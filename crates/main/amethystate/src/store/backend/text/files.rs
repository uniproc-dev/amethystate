use super::document::TextDocument;
use super::error::TextStoreError;
use super::store::{meta_at, meta_key};
use crate::errors::StorageError;
use crate::store::StorageResult;
use crate::store::config::FileWritePolicy;
use crate::store::facts::{Facts, StoreFile as StoreFileFact};
use crate::store::screening::Noticed;
use crate::store::traits::StoreLayout;
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use parking_lot::{Mutex, RwLock};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tracing::{error, warn};

#[derive(Clone)]
pub struct StoreFile<D> {
    pub path: PathBuf,
    pub backup_path: PathBuf,
    pub doc: Arc<RwLock<D>>,
    pub write_policy: FileWritePolicy,
    /// Held across rendering the document *and* replacing the file, so two
    /// flushes cannot interleave.
    ///
    /// Each replacement is atomic on its own, which buys nothing once there are
    /// two writers: the debouncer's thread and a `save_now` from anywhere would
    /// both render, then both replace, and whichever replaced second won -
    /// leaving the file holding what the *first* one saw. `save_now` returning
    /// `Ok` meant this thread's replacement landed, not that it is still there.
    flush: Arc<Mutex<()>>,

    /// The bytes this store last left in the file, as a hash of them.
    ///
    /// A watcher wakes because the file was *touched*, which is not the same as
    /// changed: an editor saving what it already held, a backup tool, a sync
    /// client stamping a time. Finding that out used to cost a parse of the
    /// whole file and two renders to compare against - three passes to learn
    /// that nothing happened.
    ///
    /// Bytes identical to the ones we wrote are content identical, so this
    /// answers before any of that. It only answers *yes*: a file rewritten with
    /// different spacing hashes differently and falls through to the reading it
    /// would have had anyway. `None` is "not known", which every failure and
    /// every open must leave behind - a stale answer here would be a file taken
    /// for ours that is somebody else's.
    wrote: Arc<Mutex<Option<u128>>>,
}

impl<D: TextDocument> StoreFile<D> {
    pub fn new(path: PathBuf, initial_doc: D, write_policy: FileWritePolicy) -> Self {
        let backup_path = StoreLayout::rewrite_copy_of(&path);
        Self {
            path,
            backup_path,
            doc: Arc::new(RwLock::new(initial_doc)),
            write_policy,
            flush: Arc::new(Mutex::new(())),
            wrote: Arc::new(Mutex::new(None)),
        }
    }

    /// Takes the copy this open would put back if its migration did not
    /// finish.
    pub fn create_backup(&self) -> StorageResult<()> {
        if self.path.exists() {
            std::fs::copy(&self.path, &self.backup_path)
                .map_err(TextStoreError::from)
                .change_context(StorageError::Open)
                .attach_store_file(&self.path)
                .attach_with(|| format!("backup: {}", self.backup_path.display()))?;
        }
        Ok(())
    }

    /// Reads the file, recovering from the copy beside it where it will not
    /// read.
    ///
    /// Nothing is copied here. The copy is taken once the whole open is known
    /// to go ahead - see [`StoreFiles::take_backups`] - because an open that is
    /// refused must leave nothing of its own behind: a `.bak` beside the store
    /// is read by the next open as an unfinished previous run, and it would
    /// recover onto it.
    pub fn read_or_recover(&self) -> StorageResult<D> {
        self.read_or_recover_unless(|_| None)
    }

    /// The same, with a second reason a document may be no good.
    ///
    /// A parse failure is not the only way a file arrives half-written: a
    /// format that calls an empty file a valid empty document parses it
    /// happily, and only something outside the file knows better. `suspect`
    /// says so, and what it names is treated exactly as a parse failure -
    /// recovered from the backup, and refused if there is none.
    pub fn read_or_recover_unless(
        &self,
        suspect: impl Fn(&D) -> Option<&'static str>,
    ) -> StorageResult<D> {
        let read = match self.lost_its_file() {
            true => Err(error_stack::Report::new(StorageError::Open)
                .attach(StoreFileFact(self.path.clone()))
                .attach("the file is gone and the copy kept for it is not")),
            false => self.load_or_empty().and_then(|doc| match suspect(&doc) {
                Some(why) => Err(error_stack::Report::new(StorageError::Open)
                    .attach(StoreFileFact(self.path.clone()))
                    .attach(why)),
                None => Ok(doc),
            }),
        };

        match read {
            Ok(doc) => Ok(doc),
            Err(unreadable) => match self.recover_from_backup() {
                Some(doc) => {
                    warn!(
                        path = %self.path.display(),
                        backup = %self.backup_path.display(),
                        "the file could not be read and was restored from the backup a \
                         previous open left behind"
                    );
                    Ok(doc)
                }
                None => Err(unreadable),
            },
        }
    }

    /// Whether the file is absent while the copy kept for it is not.
    ///
    /// An absent file is an unwritten store, which is the ordinary first open -
    /// unless a backup stands beside it, and then it is a store whose file went
    /// missing since the last open wrote one. That is the case the copy exists
    /// for, and reading it as a new store loses both: the empty document is
    /// persisted over the data file and the copy is cleaned up behind it.
    fn lost_its_file(&self) -> bool {
        !self.path.exists() && self.backup_path.exists()
    }

    fn recover_from_backup(&self) -> Option<D> {
        if !self.backup_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&self.backup_path).ok()?;
        let doc = D::parse(&content).ok()?;
        self.forget_what_the_file_holds();
        std::fs::copy(&self.backup_path, &self.path).ok()?;

        Some(doc)
    }

    pub fn load_or_empty(&self) -> StorageResult<D> {
        if self.path.exists() {
            let content = std::fs::read_to_string(&self.path)
                .map_err(TextStoreError::from)
                .change_context(StorageError::Open)
                .attach_store_file(&self.path)?;
            D::parse(&content).attach_store_file(&self.path)
        } else {
            Ok(D::empty())
        }
    }

    /// Renders the document and replaces the file with it, as one step.
    ///
    /// The lock covers both halves rather than the read alone. A guard taken
    /// only for the render is released before the replacement, which is where
    /// two flushes would cross: A renders, B renders, B replaces, A replaces,
    /// and the file ends up holding what A saw.
    pub fn persist(&self) -> StorageResult<()> {
        self.persist_while(None).map(|_| ())
    }

    /// The same, refusing to replace a file that moved since `left`. `None`
    /// has nothing to compare against and replaces.
    ///
    /// What the file holds is forgotten before the attempt rather than after
    /// it: a write that fails partway leaves a file this store cannot vouch
    /// for, and the honest answer to "is that ours" is then "not known".
    pub(crate) fn persist_while(
        &self,
        left: Option<(u64, std::time::SystemTime)>,
    ) -> StorageResult<Wrote> {
        let _flushing = self.flush.lock();

        *self.wrote.lock() = None;

        let content = self.doc.read().serialize().attach_store_file(&self.path)?;
        let still = || match left {
            Some(left) => standing_of(&self.path) == Some(left),
            None => true,
        };

        let replaced = persist_atomic(&self.path, &content, self.write_policy, &still)
            .map_err(TextStoreError::from)
            .change_context(StorageError::Flush)
            .attach_store_file(&self.path)?;

        if replaced {
            *self.wrote.lock() = Some(hash_of(&content));
        }

        Ok(match replaced {
            true => Wrote::Replaced(what_we_left(&self.path, &content)),
            false => Wrote::FileMoved,
        })
    }

    /// Whether `content` is byte for byte what this store last left in the
    /// file.
    ///
    /// Only ever *yes* with certainty: `false` covers both "somebody else wrote
    /// it" and "we do not know", and both mean read it properly.
    pub(crate) fn wrote_exactly(&self, content: &str) -> bool {
        matches!(*self.wrote.lock(), Some(held) if held == hash_of(content))
    }

    /// The file and this store now hold the same thing, and these are the bytes.
    ///
    /// Taking somebody else's edit agrees with the file as surely as writing it
    /// does, and leaving the old answer behind is how the *next* reading gets
    /// mistaken for ours: they write, we take it, they undo - and the file comes
    /// back to bytes we once wrote, which we would then wave past without
    /// looking, holding their first edit for ever.
    pub(crate) fn agrees_with(&self, content: &str) {
        *self.wrote.lock() = Some(hash_of(content));
    }

    /// Nothing is known about the file's contents any more.
    pub(crate) fn forget_what_the_file_holds(&self) {
        *self.wrote.lock() = None;
    }

    /// Puts the file back the way this open found it, and says so when it
    /// cannot.
    ///
    /// Nothing is returned because the caller is already carrying the failure
    /// that brought it here, and there is no answer to a restore that will not
    /// land. There is a report, though: this is the one path that leaves the
    /// file holding what a half-finished open wrote.
    pub fn restore_from_backup(&self, fallback_to_initial: &D) {
        *self.doc.write() = fallback_to_initial.clone();

        self.forget_what_the_file_holds();

        if self.backup_path.exists() {
            if let Err(io) = std::fs::copy(&self.backup_path, &self.path) {
                error!(
                    file = %self.path.display(),
                    backup = %self.backup_path.display(),
                    error = %io,
                    "the copy taken at the start of this open could not be put back, so the \
                     file holds what the open that failed had written"
                );
                return;
            }

            self.remove_backup("after putting it back");
        } else if self.path.exists()
            && let Err(io) = std::fs::remove_file(&self.path)
        {
            error!(
                file = %self.path.display(),
                error = %io,
                "this open created the file and could not take it away again, so a store \
                 that was never opened is left on disk"
            );
        }
    }

    pub fn clean_backup(&self) {
        if self.backup_path.exists() {
            self.remove_backup("after the open went through");
        }
    }

    /// A copy left behind is read by the next open as an unfinished previous
    /// run, so failing to take one away is worth a line.
    fn remove_backup(&self, when: &'static str) {
        if let Err(io) = std::fs::remove_file(&self.backup_path) {
            warn!(
                file = %self.path.display(),
                backup = %self.backup_path.display(),
                error = %io,
                "the copy beside the store could not be removed {when}, and the next open \
                 reads one as an unfinished previous run"
            );
        }
    }
}

#[derive(Clone)]
pub struct StoreFiles<D: TextDocument> {
    pub data: StoreFile<D>,
    pub meta: StoreFile<D>,
}

/// Where the metadata records that the data file held something.
///
/// The one fact that tells a store somebody emptied from a file caught
/// half-written: both are a document with no keys, and the file cannot say
/// which it is. TOML shows it plainest - an empty file is a valid empty
/// document - but the question is not toml's, and neither is the answer.
///
/// It lives in the metadata because it is the store's opinion of itself.
/// Writing it into the data file would put it where a person edits, where they
/// would rightly delete it, and where a store that was cleared would stop being
/// empty.
fn held_key() -> StorePath {
    meta_at(&meta_key("held", &StorePath::root()))
}

/// Whether a document holds no key at all, which is what a store somebody
/// emptied and a file caught half-written both look like.
///
/// A document that will not even be scanned is not called empty: the question
/// here is only whether it is, and anything else is somebody else's failure to
/// report.
pub(super) fn has_no_keys<D: TextDocument>(doc: &D) -> bool {
    doc.scan_keys(&StorePath::root())
        .map(|children| children.is_empty())
        .unwrap_or(false)
}

impl<D: TextDocument> StoreFiles<D> {
    /// Both documents, and the data one checked against what the metadata
    /// remembers of it.
    ///
    /// The metadata is read first because it is what judges the data: a
    /// document with no keys where the last save had some is a file that was
    /// truncated between then and now, and taking it at face value would save
    /// the emptiness back over everything. It is reported after the data,
    /// though: a metadata file that will not read is no reason for the data to
    /// go unread.
    ///
    /// Nothing is copied here - see [`StoreFiles::take_backups`].
    pub fn load(&self) -> StorageResult<(D, D)> {
        let meta = self.meta.read_or_recover();

        let held = matches!(
            meta.as_ref()
                .ok()
                .and_then(|held: &D| held.get(&held_key()))
                .map(D::deserialize_node::<bool>),
            Some(Ok(true))
        );

        let data = self
            .data
            .read_or_recover_unless(|doc: &D| match held && has_no_keys(doc) {
                true => Some("the last save left keys here and the file now holds none"),
                false => None,
            })
            .attach("role: the store's data")?;

        let meta = meta.attach("role: the store's schema bookkeeping")?;

        Ok((data, meta))
    }

    /// Takes the copies the migration pass would be put back to, immediately
    /// before it runs.
    ///
    /// Late on purpose. A copy beside the store is read by the next open as a
    /// previous open that did not finish, so every way this open can still be
    /// refused - a format record this build cannot honour, a store that will
    /// not build - has to come first: an operation that did not happen leaves
    /// nothing of its own.
    pub fn take_backups(&self) -> StorageResult<()> {
        self.data.create_backup().attach("role: the store's data")?;
        self.meta
            .create_backup()
            .attach("role: the store's schema bookkeeping")
    }

    /// Writes the metadata first, and the fact about the data before the data.
    ///
    /// A crash between the two leaves the metadata saying more than the file
    /// does, which costs a refusal the backup answers. The other order costs
    /// the data: an emptied store whose metadata still says it held keys reads
    /// as truncated for good.
    pub fn persist(&self) -> StorageResult<()> {
        self.persist_while(None).map(|_| ())
    }

    /// The same, with the data file refusing to replace one that moved since
    /// `left`. The metadata is written either way.
    pub(crate) fn persist_while(
        &self,
        left: Option<(u64, std::time::SystemTime)>,
    ) -> StorageResult<Wrote> {
        self.remember_what_the_data_holds()?;

        self.meta
            .persist()
            .attach("role: the store's schema bookkeeping")?;
        self.data
            .persist_while(left)
            .attach("role: the store's data")
    }

    fn remember_what_the_data_holds(&self) -> StorageResult<()> {
        let holds = !has_no_keys(&*self.data.doc.read());
        let key = held_key();

        {
            let mut guard = self.meta.doc.write();
            let said = matches!(
                guard.get(&key).map(D::deserialize_node::<bool>),
                Some(Ok(true))
            );

            if said == holds {
                return Ok(());
            }

            let node = D::serialize_node(&holds, &Noticed::unlimited())
                .change_context(StorageError::Meta)
                .attach_key(&key)?;

            guard
                .set(&key, node)
                .change_context(StorageError::Meta)
                .attach_key(&key)?;
        }

        Ok(())
    }

    /// Takes away what an open no longer needs and what an earlier crash left:
    /// the copies this open took, and any temporary a killed write abandoned
    /// beside either file.
    pub fn clean_backups(&self) {
        self.data.clean_backup();
        self.meta.clean_backup();
        sweep_temporaries(&self.data.path);
        sweep_temporaries(&self.meta.path);
    }

    pub fn restore_from_backups(&self, fallback_data: &D, fallback_meta: &D) {
        self.data.restore_from_backup(fallback_data);
        self.meta.restore_from_backup(fallback_meta);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wrote {
    /// The file now holds what was written, and this is how it stood the
    /// instant after - taken while the replacement still holds the flush lock,
    /// because a stat taken any later can be somebody else's.
    Replaced(Option<(u64, std::time::SystemTime)>),
    FileMoved,
}

/// A file's bytes as one number, for telling "the same file" from "the same
/// size and time".
pub(super) fn hash_of(content: &str) -> u128 {
    xxhash_rust::xxh3::xxh3_128(content.as_bytes())
}

pub(super) fn standing_of(file: &Path) -> Option<(u64, std::time::SystemTime)> {
    let held = std::fs::metadata(file).ok()?;
    Some((held.len(), held.modified().ok()?))
}

/// How the file stands now, if what stands there is what we just put in it.
///
/// The stat cannot be taken with the rename, so between the two somebody else's
/// replacement can land - and taking that stat as ours is how the next save
/// comes to believe the file is as it left it and pours the document over an
/// edit it never read. Answering `None` there says *I do not know how I left
/// it*, which asks the next save to read the file rather than replace it.
///
/// What the length proves is one-sided: bytes of the same length in the window
/// still read as ours, and that is the narrow case this does not close.
fn what_we_left(file: &Path, content: &str) -> Option<(u64, std::time::SystemTime)> {
    standing_of(file).filter(|(len, _)| *len == content.len() as u64)
}

/// Writes `content` where `path` names, so that a reader sees either the whole
/// of it or none.
///
/// The temporary file is made in the target's own directory, because a
/// replacement has to sit on the same volume, and the contents are flushed
/// before the name is moved: otherwise the rename can reach the disk while the
/// bytes are still in the write-back cache, which is how a config file comes
/// back truncated after a power cut. Windows offers no write-through on the
/// replacement itself, so the flush has to be ours.
///
/// A replacement that has to be retried takes the same temporary file back from
/// the failure and tries again with it: the contents are written and flushed
/// already, and only the name is in dispute.
///
/// How long each of the two steps is worth is [`FileWritePolicy`], because what
/// is holding the file is the application's business and not this function's.
fn persist_atomic(
    path: &Path,
    content: &str,
    policy: FileWritePolicy,
    still: &dyn Fn() -> bool,
) -> io::Result<bool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut written = None;
    for attempt in 0..policy.write.attempts.max(1) {
        match write_temp(path, content) {
            Ok(tmp) => {
                written = Some(tmp);
                break;
            }
            Err(e) if attempt + 1 >= policy.write.attempts => return Err(e),
            Err(_) => std::thread::sleep(policy.write.pause),
        }
    }
    let mut tmp = written.expect("the loop above returns rather than falling through");

    if !still() {
        return Ok(false);
    }

    for attempt in 0..policy.replace.attempts.max(1) {
        match tmp.persist(path) {
            Ok(_) => return Ok(true),
            Err(e) if attempt + 1 >= policy.replace.attempts => return Err(e.error),
            Err(e) => {
                tmp = e.file;
                std::thread::sleep(policy.replace.pause);
            }
        }
    }
    unreachable!("the loop above returns on its last attempt")
}

/// The contents in a file of their own, beside the target and already on the
/// disk.
fn write_temp(target: &Path, content: &str) -> io::Result<NamedTempFile> {
    let dir = target.parent().unwrap_or(Path::new("."));

    // No random part of `tempfile`'s beside the mark: the nonce inside it is
    // already thirty-two bits of one, and a name that does collide comes back
    // as `AlreadyExists` - which the loop above retries with a fresh mark.
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!("{}{}", temporaries_of(target), a_mark_of_ours()))
        .suffix(TEMPORARY)
        .rand_bytes(0)
        .tempfile_in(dir)?;

    tmp.write_all(content.as_bytes())?;
    tmp.as_file().sync_all()?;
    Ok(tmp)
}

const TEMPORARY: &str = ".tmp";

/// What the mark on a temporary is worked out against.
///
/// Not a secret and not pretending to be one - it is in the binary and anyone
/// can spell a name that verifies. What it rules out is the accident: a file
/// left beside the store whose name happens to take the same shape, which a
/// sweep matching on shape alone would take away.
const WRITTEN_BY_US: u64 = 0x616d_6574_6879_7374;

/// A fresh mark: a nonce and what this library makes of it.
///
/// Only the pair goes in the name, so a sweep can work the second half out of
/// the first and see whether it agrees.
fn a_mark_of_ours() -> String {
    let nonce = uuid::Uuid::new_v4().as_u128() as u32;
    format!("{nonce:08x}{:08x}", vouched_for(nonce))
}

fn vouched_for(nonce: u32) -> u32 {
    let mut bytes = [0u8; 12];
    bytes[..4].copy_from_slice(&nonce.to_le_bytes());
    bytes[4..].copy_from_slice(&WRITTEN_BY_US.to_le_bytes());

    xxhash_rust::xxh3::xxh3_64(&bytes) as u32
}

/// Whether `mark` is one this library wrote, rather than a name that looks
/// like one.
fn is_a_mark_of_ours(mark: &str) -> bool {
    if mark.len() != 16 {
        return false;
    }

    let (nonce, claimed) = mark.split_at(8);
    let Ok(nonce) = u32::from_str_radix(nonce, 16) else {
        return false;
    };
    let Ok(claimed) = u32::from_str_radix(claimed, 16) else {
        return false;
    };

    vouched_for(nonce) == claimed
}

/// What a temporary standing in for `target` is called, before the mark and the
/// random part.
///
/// Named after the file it is going to become rather than at random, so a
/// leftover says which store it belongs to - and so [`sweep_temporaries`] can
/// take away this store's and nobody else's.
fn temporaries_of(target: &Path) -> String {
    let named = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    format!("{named}.")
}

/// Takes away the temporaries a killed write left beside `target`.
///
/// A replacement is a temporary file written and flushed, then renamed. A
/// process killed between the two cannot come back for it, so one is left per
/// crash - each a whole copy of the document, which for a store is also a copy
/// of whatever was in it. Nothing else collects them.
///
/// Three things have to agree before one goes: it is named after this file, it
/// ends the way a temporary does, and it carries a mark this library can work
/// out for itself. The name alone is a convention, and a convention is shared
/// with whoever else writes beside the store.
///
/// A failure is not reported: on Windows a temporary another process is still
/// writing is locked, so this passes it over, which is the answer wanted
/// anyway. The one it cannot remove is the one still in use.
fn sweep_temporaries(target: &Path) {
    let Some(dir) = target.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let prefix = temporaries_of(target);

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(mark) = rest.strip_suffix(TEMPORARY) else {
            continue;
        };

        if is_a_mark_of_ours(mark) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::super::json::json_doc::JsonDocument;
    use super::*;
    use amethystate_core::test_utils::TempPath;

    fn holding(at: &Path, what: &str) -> StoreFile<JsonDocument> {
        let doc = JsonDocument::parse(what).unwrap();
        StoreFile::new(at.to_path_buf(), doc, FileWritePolicy::default())
    }

    #[test]
    fn a_mark_this_library_wrote_reads_back_as_its_own() {
        for _ in 0..64 {
            assert!(is_a_mark_of_ours(&a_mark_of_ours()));
        }
    }

    #[test]
    fn a_name_of_the_same_shape_is_not_a_mark() {
        assert!(
            !is_a_mark_of_ours("0123456789abcdef"),
            "sixteen hex characters are a shape, and a sweep that takes a shape takes \
             whatever else happens to have it"
        );
        assert!(!is_a_mark_of_ours("draft"), "a name somebody typed");
        assert!(!is_a_mark_of_ours(""), "nothing at all");
        assert!(
            !is_a_mark_of_ours("00000000ffffffff"),
            "a nonce with the wrong answer beside it"
        );
    }

    #[test]
    fn a_mark_with_one_character_changed_stops_being_one() {
        let mark = a_mark_of_ours();
        let mut bent: Vec<char> = mark.chars().collect();
        bent[3] = if bent[3] == 'a' { 'b' } else { 'a' };

        assert!(!is_a_mark_of_ours(&bent.into_iter().collect::<String>()));
    }

    #[test]
    fn a_sweep_takes_this_library_s_leftovers_and_leaves_everything_else() {
        let at = TempPath::new("sweeping");
        let dir = at
            .path()
            .parent()
            .expect("a temporary has a directory")
            .to_path_buf();
        let data = dir.join("settings.json");

        let ours = dir.join(format!("settings.json.{}{TEMPORARY}", a_mark_of_ours()));
        let shaped_the_same = dir.join(format!("settings.json.0123456789abcdef{TEMPORARY}"));
        let somebody_elses = dir.join(format!("settings.json.draft{TEMPORARY}"));
        let another_store = dir.join(format!("other.json.{}{TEMPORARY}", a_mark_of_ours()));

        for file in [
            &data,
            &ours,
            &shaped_the_same,
            &somebody_elses,
            &another_store,
        ] {
            std::fs::write(file, "x").unwrap();
        }

        sweep_temporaries(&data);

        assert!(
            !ours.exists(),
            "the leftover a killed write of ours left is still there"
        );
        assert!(data.exists(), "the sweep took the store's own file");
        assert!(
            shaped_the_same.exists(),
            "a name that merely takes the shape of ours was taken away"
        );
        assert!(somebody_elses.exists());
        assert!(
            another_store.exists(),
            "the sweep reached past its own file into another store's leftovers"
        );

        for file in [&data, &shaped_the_same, &somebody_elses, &another_store] {
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    fn a_file_that_moved_since_it_was_read_is_left_alone() {
        let at = TempPath::new("persist_while");
        let file = holding(at.path(), r#"{"ours":1}"#);

        file.persist().unwrap();
        let read_at = standing_of(at.path());

        std::fs::write(at.path(), r#"{"theirs":2}"#).unwrap();

        assert_eq!(
            file.persist_while(read_at).unwrap(),
            Wrote::FileMoved,
            "the file was written by somebody else after it was read"
        );
        assert_eq!(
            std::fs::read_to_string(at.path()).unwrap(),
            r#"{"theirs":2}"#,
            "the replacement went ahead over a file this save had never seen"
        );

        assert!(
            matches!(
                file.persist_while(standing_of(at.path())).unwrap(),
                Wrote::Replaced(_)
            ),
            "the same save against the file as it stands now must land"
        );
        assert!(
            std::fs::read_to_string(at.path()).unwrap().contains("ours"),
            "the replacement that was allowed did not happen"
        );
    }
}
