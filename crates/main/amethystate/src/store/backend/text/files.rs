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
    /// would have had anyway. `None` is "not known", which every failure must
    /// leave behind - a stale answer here would be a file taken for ours that is
    /// somebody else's. An open knows exactly: it holds what it read, or what
    /// it wrote.
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
    /// to go ahead - see [`StoreFiles::write_what_the_open_changed`] - because an open that is
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
            false => self
                .read_as_found()
                .and_then(|(doc, content)| match suspect(&doc) {
                    Some(why) => Err(error_stack::Report::new(StorageError::Open)
                        .attach(StoreFileFact(self.path.clone()))
                        .attach(why)),
                    None => Ok((doc, content)),
                }),
        };

        match read {
            Ok((doc, content)) => {
                if let Some(content) = content {
                    self.agrees_with(&content);
                }
                Ok(doc)
            }
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

    /// The file's document, or an empty one where there is no file, and
    /// whether the file is byte for byte what this store last left in it.
    pub(crate) fn load_or_empty_as_left(&self) -> StorageResult<(D, bool)> {
        self.read_as_found().map(|(doc, content)| {
            let ours = content.is_some_and(|content| self.wrote_exactly(&content));
            (doc, ours)
        })
    }

    fn read_as_found(&self) -> StorageResult<(D, Option<String>)> {
        if !self.path.exists() {
            return Ok((D::empty(), None));
        }

        let content = std::fs::read_to_string(&self.path)
            .map_err(TextStoreError::from)
            .change_context(StorageError::Open)
            .attach_store_file(&self.path)?;
        let doc = D::parse(&content).attach_store_file(&self.path)?;
        Ok((doc, Some(content)))
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
    pub(crate) fn persist_while(&self, left: Option<Standing>) -> StorageResult<Wrote> {
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

    /// Replaces the file with `doc`, leaving what this store holds in memory
    /// as it is.
    pub(crate) fn persist_document(&self, doc: &D) -> StorageResult<()> {
        let _flushing = self.flush.lock();

        *self.wrote.lock() = None;

        let content = doc.serialize().attach_store_file(&self.path)?;
        persist_atomic(&self.path, &content, self.write_policy, &|| true)
            .map_err(TextStoreError::from)
            .change_context(StorageError::Flush)
            .attach_store_file(&self.path)?;

        *self.wrote.lock() = Some(hash_of(&content));
        Ok(())
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

    /// Puts the file back the way this open found it, and answers whether it
    /// is.
    ///
    /// The caller is already carrying the failure that brought it here, and
    /// there is no answer to a restore that will not land - but what it tells
    /// its own caller about the file depends on this, and a report that says
    /// the file was put back when it was not sends a reader past the one file
    /// they should look at. What went wrong is logged here, with the paths.
    pub fn restore_from_backup(&self, fallback_to_initial: &D) -> bool {
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
                return false;
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
            return false;
        }

        true
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

/// Where the metadata records a write an open has under way.
fn migrating_key() -> StorePath {
    meta_at(&meta_key("migrating", &StorePath::root()))
}

/// A write an open has under way: the metadata it is to become, and hashes of
/// the data it is writing and of the data it found - empty where there was none.
#[derive(serde::Serialize, serde::Deserialize)]
struct Migrating {
    meta: String,
    data: String,
    was: String,
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
    /// Nothing is copied here - see [`StoreFiles::write_what_the_open_changed`].
    pub fn load(&self) -> StorageResult<(D, D)> {
        let meta = self
            .meta
            .read_or_recover()
            .and_then(|found| self.settle_a_write_under_way(found));

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

    /// Writes the metadata first, and the fact about the data before the data,
    /// with the data file refusing to replace one that moved since `left`. The
    /// metadata is written either way.
    ///
    /// A crash between the two leaves the metadata saying more than the file
    /// does, which costs a refusal the backup answers. The other order costs
    /// the data: an emptied store whose metadata still says it held keys reads
    /// as truncated for good.
    pub(crate) fn persist_while(&self, left: Option<Standing>) -> StorageResult<Wrote> {
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

    /// The metadata as found, once a write a stopped open left under way is
    /// settled - see [`StoreFiles::write_both`].
    ///
    /// Data that is what the open was writing finishes it: the metadata it was
    /// about to become is written. Data that is what the open found drops the
    /// record, and the migration runs again over it. Anything else is
    /// refused, because which data the metadata describes cannot be told.
    fn settle_a_write_under_way(&self, found: D) -> StorageResult<D> {
        let record: Migrating = match found.get(&migrating_key()) {
            None => return Ok(found),
            Some(node) => D::deserialize_node(node)
                .change_context(StorageError::Meta)
                .attach_key(&migrating_key())?,
        };

        let now = standing_of(&self.data.path)
            .map(|(_, _, held)| format!("{held:032x}"))
            .unwrap_or_default();

        let settled = if now == record.data {
            warn!(
                path = %self.data.path.display(),
                "an open stopped after writing a migrated store's data and before recording \
                 it, and the record it left finishes the metadata"
            );
            D::parse(&record.meta).attach_store_file(&self.meta.path)?
        } else if now == record.was {
            warn!(
                path = %self.data.path.display(),
                "an open stopped before writing a migrated store's data, so the migration runs \
                 again"
            );
            let mut dropped = found.clone();
            dropped
                .delete(&migrating_key())
                .change_context(StorageError::Meta)
                .attach_key(&migrating_key())?;
            dropped
        } else {
            return Err(error_stack::Report::new(StorageError::Open)
                .attach(StoreFileFact(self.data.path.clone()))
                .attach(
                    "an open stopped while writing a migration here, and the data file is \
                     neither what it found nor what it was writing, so which data the metadata \
                     describes cannot be told",
                ));
        };

        self.meta
            .persist_document(&settled)
            .attach("role: the store's schema bookkeeping")?;
        self.data.clean_backup();
        self.meta.clean_backup();

        Ok(settled)
    }

    /// Writes both files of an open that changed both, so that a stop between
    /// them is finished or undone by the next open rather than read as either
    /// half.
    ///
    /// Each file is one atomic replace and nothing joins the two. So the
    /// metadata goes first as it was found, with a record of the write under
    /// way: the metadata it is about to become, and a hash of the data being
    /// written and of the data found. Then the data, then the metadata itself.
    /// [`StoreFiles::load`] reads the record back.
    ///
    /// The copies go as soon as the data has landed. From there the record
    /// answers a stop, and a copy left behind would hold the data from before
    /// the migration beside metadata that says it ran.
    fn write_both(&self, found_data: &D, found_meta: &D) -> StorageResult<()> {
        let finished = self
            .meta
            .doc
            .read()
            .serialize()
            .attach_store_file(&self.meta.path)?;
        let writing = self
            .data
            .doc
            .read()
            .serialize()
            .attach_store_file(&self.data.path)?;

        let record = Migrating {
            meta: finished,
            data: format!("{:032x}", hash_of(&writing)),
            was: standing_of(&self.data.path)
                .map(|(_, _, held)| format!("{held:032x}"))
                .unwrap_or_default(),
        };
        let node = D::serialize_node(&record, &Noticed::unlimited())
            .change_context(StorageError::Meta)
            .attach_key(&migrating_key())?;
        let mut under_way = found_meta.clone();
        under_way
            .set(&migrating_key(), node)
            .change_context(StorageError::Meta)
            .attach_key(&migrating_key())?;

        self.meta
            .create_backup()
            .attach("role: the store's schema bookkeeping")?;
        self.data.create_backup().attach("role: the store's data")?;

        let begun = self
            .meta
            .persist_document(&under_way)
            .attach("role: the store's schema bookkeeping")
            .inspect(|()| stop_the_open_after("meta"))
            .and_then(|()| {
                self.data
                    .persist()
                    .attach("role: the store's data")
                    .inspect(|()| stop_the_open_after("data"))
            });

        if let Err(why) = begun {
            let meta_back = self.meta.restore_from_backup(found_meta);
            let data_back = self.data.restore_from_backup(found_data);

            return Err(why.attach(match meta_back && data_back {
                true => "the files this open wrote were put back from their copies",
                false => {
                    "the files this open wrote could not all be put back from their copies, \
                     and hold what it wrote - the log names which"
                }
            }));
        }

        self.data.clean_backup();
        self.meta.clean_backup();

        self.meta
            .persist()
            .attach("role: the store's schema bookkeeping")
            .attach(
                "the data landed and the metadata still records the write under way, which \
                 the next open finishes",
            )?;

        sweep_temporaries(&self.data.path);
        sweep_temporaries(&self.meta.path);

        Ok(())
    }

    /// Writes the files this open changed, and leaves alone the ones it only
    /// read.
    ///
    /// Another store can hold the same file and commit to it while this one
    /// reads, migrates and settles. Putting back a document the open did not
    /// change pours the copy read at the start over whatever that store
    /// committed since; the next save here lays its own writes over the file
    /// instead. A file not there yet is written either way, so an open leaves a
    /// store behind.
    ///
    /// Each file about to be written is copied first and the copy taken away
    /// once both land, so a `.bak` beside the store is an open whose writing did
    /// not finish. Nothing else is copied or cleaned: another open's copies
    /// stand under the same names, and they are its own. Temporaries a killed
    /// write abandoned beside either file go on the way out.
    pub fn write_what_the_open_changed(&self, found_data: &D, found_meta: &D) -> StorageResult<()> {
        self.remember_what_the_data_holds()?;

        let meta = changed(&self.meta, found_meta)?;
        let data = changed(&self.data, found_data)?;

        if meta && data {
            return self.write_both(found_data, found_meta);
        }

        if meta {
            self.meta
                .create_backup()
                .attach("role: the store's schema bookkeeping")?;
        }
        if data {
            self.data.create_backup().attach("role: the store's data")?;
        }

        let written = match meta {
            true => self
                .meta
                .persist()
                .attach("role: the store's schema bookkeeping")
                .inspect(|()| stop_the_open_after("meta")),
            false => Ok(()),
        }
        .and_then(|()| match data {
            true => self
                .data
                .persist()
                .attach("role: the store's data")
                .inspect(|()| stop_the_open_after("data")),
            false => Ok(()),
        });

        if let Err(why) = written {
            let meta_back = !meta || self.meta.restore_from_backup(found_meta);
            let data_back = !data || self.data.restore_from_backup(found_data);

            return Err(why.attach(match meta_back && data_back {
                true => "the files this open wrote were put back from their copies",
                false => {
                    "the files this open wrote could not all be put back from their copies, \
                     and hold what it wrote - the log names which"
                }
            }));
        }

        if meta {
            self.meta.clean_backup();
        }
        if data {
            self.data.clean_backup();
        }
        sweep_temporaries(&self.data.path);
        sweep_temporaries(&self.meta.path);

        Ok(())
    }
}

/// Ends the process where a test asked, so a stop between the open's writes can
/// be made to happen on purpose.
#[cfg(feature = "test-utils")]
fn stop_the_open_after(written: &str) {
    if std::env::var("AMETHYSTATE_STOP_THE_OPEN_AFTER").as_deref() == Ok(written) {
        std::process::abort();
    }
}

#[cfg(not(feature = "test-utils"))]
fn stop_the_open_after(_written: &str) {}

/// Whether `file` holds something other than `found`, or is not there yet.
fn changed<D: TextDocument>(file: &StoreFile<D>, found: &D) -> StorageResult<bool> {
    if !file.path.exists() {
        return Ok(true);
    }

    let now = file.doc.read().serialize().attach_store_file(&file.path)?;
    let then = found.serialize().attach_store_file(&file.path)?;

    Ok(now != then)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wrote {
    /// The file now holds what was written, and this is how it stood the
    /// instant after - taken while the replacement still holds the flush lock,
    /// because a stat taken any later can be somebody else's.
    Replaced(Option<Standing>),
    FileMoved,
}

/// A file's bytes as one number, for telling "the same file" from "the same
/// size and time".
pub(super) fn hash_of(content: &str) -> u128 {
    xxhash_rust::xxh3::xxh3_128(content.as_bytes())
}

/// How a file stood: its length, when it was last modified, and a hash of its
/// bytes.
///
/// The hash is what answers. A replacement of the same length can land within
/// the clock's resolution, or keep the time it was copied with, and the first
/// two then say nothing moved while the bytes are somebody else's.
pub(crate) type Standing = (u64, std::time::SystemTime, u128);

pub(super) fn standing_of(file: &Path) -> Option<Standing> {
    let held = std::fs::metadata(file).ok()?;
    let modified = held.modified().ok()?;
    let content = std::fs::read(file).ok()?;

    Some((held.len(), modified, xxhash_rust::xxh3::xxh3_128(&content)))
}

/// How the file stands now, if what stands there is what we just put in it.
///
/// The stat cannot be taken with the rename, so between the two somebody else's
/// replacement can land - and taking that stat as ours is how the next save
/// comes to believe the file is as it left it and pours the document over an
/// edit it never read. Answering `None` there says *I do not know how I left
/// it*, which asks the next save to read the file rather than replace it.
fn what_we_left(file: &Path, content: &str) -> Option<Standing> {
    standing_of(file).filter(|(_, _, held)| *held == hash_of(content))
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
/// Four things have to agree before one goes: it is named after this file, it
/// ends the way a temporary does, it carries a mark this library can work out
/// for itself, and it has stood untouched for [`ABANDONED_AFTER`]. The name
/// alone is a convention, and a convention is shared with whoever else writes
/// beside the store.
///
/// The age is what tells a leftover from a write still under way. Another
/// store on the same file, in this process or another, writes its replacement
/// beside it, and nothing locks that file against removal while it does:
/// taking a young one fails that write with a file that is not there.
///
/// A failure to remove one is not reported; the next open tries again.
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

        if is_a_mark_of_ours(mark) && abandoned(&entry) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// How long a temporary stands untouched before a sweep takes it for one a
/// killed write abandoned: far past any replacement still being retried.
const ABANDONED_AFTER: std::time::Duration = std::time::Duration::from_secs(600);

fn abandoned(entry: &std::fs::DirEntry) -> bool {
    entry
        .metadata()
        .and_then(|held| held.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= ABANDONED_AFTER)
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
            dated_back(file, std::time::Duration::from_secs(3600));
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

    fn dated_back(file: &Path, by: std::time::Duration) {
        std::fs::File::options()
            .write(true)
            .open(file)
            .and_then(|held| held.set_modified(std::time::SystemTime::now() - by))
            .unwrap();
    }

    #[test]
    fn a_temporary_too_young_to_be_abandoned_is_left_to_its_writer() {
        let at = TempPath::new("sweeping_young");
        let dir = at
            .path()
            .parent()
            .expect("a temporary has a directory")
            .to_path_buf();
        let data = dir.join("settings.json");
        let in_flight = dir.join(format!("settings.json.{}{TEMPORARY}", a_mark_of_ours()));

        std::fs::write(&in_flight, "x").unwrap();

        sweep_temporaries(&data);

        assert!(
            in_flight.exists(),
            "another store's replacement still under way was taken from under it"
        );

        let _ = std::fs::remove_file(&in_flight);
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

    #[test]
    fn a_file_rewritten_to_the_same_length_and_time_is_left_alone_too() {
        let at = TempPath::new("persist_same_standing");
        let file = holding(at.path(), r#"{"ours":1}"#);

        file.persist().unwrap();
        let read_at = standing_of(at.path());
        let modified = std::fs::metadata(at.path()).unwrap().modified().unwrap();

        let theirs = std::fs::read_to_string(at.path())
            .unwrap()
            .replace("ours", "them");
        std::fs::write(at.path(), &theirs).unwrap();
        std::fs::File::options()
            .write(true)
            .open(at.path())
            .and_then(|held| held.set_modified(modified))
            .unwrap();

        assert_eq!(
            file.persist_while(read_at).unwrap(),
            Wrote::FileMoved,
            "somebody else wrote bytes of the same length and the clock says nothing moved"
        );
        assert_eq!(std::fs::read_to_string(at.path()).unwrap(), theirs);
    }

    #[test]
    fn a_restore_that_puts_the_copy_back_says_it_did() {
        let at = TempPath::new("restore_lands");
        let file = holding(at.path(), r#"{"ours":1}"#);
        std::fs::write(at.path(), r#"{"half":2}"#).unwrap();
        std::fs::write(&file.backup_path, r#"{"ours":1}"#).unwrap();

        assert!(file.restore_from_backup(&JsonDocument::parse(r#"{"ours":1}"#).unwrap()));
        assert_eq!(std::fs::read_to_string(at.path()).unwrap(), r#"{"ours":1}"#);
    }

    #[test]
    fn a_restore_that_cannot_put_the_copy_back_says_it_did_not() {
        let at = TempPath::new("restore_blocked");
        let file = holding(at.path(), r#"{"ours":1}"#);
        std::fs::write(&file.backup_path, r#"{"ours":1}"#).unwrap();
        std::fs::create_dir(at.path()).unwrap();

        assert!(!file.restore_from_backup(&JsonDocument::parse(r#"{"ours":1}"#).unwrap()));
    }
}
