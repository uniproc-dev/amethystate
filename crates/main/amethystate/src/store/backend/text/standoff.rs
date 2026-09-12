use super::document::{Navigable, TextDocument};
use super::files::{StoreFiles, Wrote, has_no_keys, standing_of};
use super::store::diff_documents;
use crate::errors::StorageError;
use crate::store::backend::utils;
use crate::store::facts::StoreFile as StoreFileFact;
use crate::store::{StorageResult, StoreEvent, SubscriptionEntry, WhenItWillNotRead};
use amethystate_core::path::StorePath;
use parking_lot::{Mutex, RwLock};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

/// Where this store wrote or swept, as the document addresses it.
#[derive(Default)]
pub(crate) struct Touched {
    at: std::collections::HashSet<StorePath>,
    under: Vec<StorePath>,

    /// The same writes as the caller named them, which is what a caller is
    /// told about rather than where the node sits in the document.
    named: std::collections::BTreeSet<StorePath>,
}

impl Touched {
    fn absorb(&mut self, other: Touched) {
        self.at.extend(other.at);
        self.under.extend(other.under);
        self.named.extend(other.named);
    }
}

/// What this store wrote that the file was not given, and what the file holds
/// that this store did not take.
#[derive(Default)]
pub(crate) struct Standoff {
    held: AtomicU64,
    merged: AtomicU64,
    touched: Mutex<Touched>,
    left: Mutex<Option<(u64, std::time::SystemTime)>>,
    saving: Mutex<()>,

    /// When a save first met a file it could not parse, so
    /// [`WhenItWillNotRead::TryAgainFor`] can tell a file that is busy from one
    /// that is broken.
    ///
    /// Cleared the moment the file reads again, so a second editor's keystroke
    /// a minute later gets the whole window over again rather than the
    /// remainder of somebody else's.
    unreadable_since: Mutex<Option<std::time::Instant>>,
}

impl Standoff {
    pub(super) fn hold(&self) {
        self.held.fetch_add(1, Ordering::Release);
    }

    pub(super) fn wrote(&self, at: &StorePath, named: &StorePath) {
        let mut touched = self.touched.lock();
        touched.at.insert(at.clone());
        touched.named.insert(named.clone());
    }

    pub(super) fn swept(&self, under: &StorePath) {
        let mut touched = self.touched.lock();
        touched.under.push(under.clone());
        touched.named.insert(under.clone());
    }

    /// What this store has written and not saved, as the caller named it.
    pub(super) fn unsaved(&self) -> Vec<StorePath> {
        self.touched.lock().named.iter().cloned().collect()
    }

    fn left(&self) -> Option<(u64, std::time::SystemTime)> {
        *self.left.lock()
    }

    /// How long the file has been unreadable, counting this meeting as the
    /// first if nothing has met one yet.
    fn unreadable_for(&self) -> std::time::Duration {
        let mut since = self.unreadable_since.lock();
        since.get_or_insert_with(std::time::Instant::now).elapsed()
    }

    /// The file parses again, so the next one that does not starts its own
    /// window.
    fn reads_again(&self) {
        *self.unreadable_since.lock() = None;
    }

    fn holding(&self, file: &Path) -> bool {
        if self.held.load(Ordering::Acquire) != self.merged.load(Ordering::Acquire) {
            return true;
        }

        match (self.left(), standing_of(file)) {
            (Some(left), Some(now)) => left != now,

            // Nothing recorded is not the same as nothing changed. A store
            // that has not written yet knows nothing about how the file
            // stands, and reading that as "it is as I left it" is how a store
            // holding one unflushed write pours its document over what another
            // one committed in the meantime.
            (None, Some(_)) => true,

            // No file to lay anything over.
            (_, None) => false,
        }
    }

    fn taking(&self) -> Touched {
        std::mem::take(&mut *self.touched.lock())
    }

    fn put_back(&self, mine: Touched) {
        self.touched.lock().absorb(mine);
    }

    /// How the file stood when this store last left it, as the write that left
    /// it saw it.
    ///
    /// Handed in rather than looked up, because a stat taken after the write
    /// released its lock can be somebody else's: adopting theirs as ours makes
    /// every later look say the file has not moved, and the next save writes
    /// the document over their edit without ever reading it.
    fn left_it(&self, standing: Option<(u64, std::time::SystemTime)>) {
        *self.left.lock() = standing;
    }
}

const SAVES: usize = 3;

pub(super) fn save<D: TextDocument>(
    files: &StoreFiles<D>,
    subscriptions: &RwLock<Vec<SubscriptionEntry>>,
    writes: &AtomicU64,
    persisted: &AtomicU64,
    standoff: &Standoff,
    settled: &AtomicU64,
    will_not_read: WhenItWillNotRead,
) -> StorageResult<()> {
    let _one_at_a_time = standoff.saving.lock();

    for _ in 0..SAVES {
        let saving = writes.load(Ordering::Acquire);
        let laid = standoff.held.load(Ordering::Acquire);
        let mine = standoff.taking();
        let mut unmoved = standoff.left();

        if standoff.holding(&files.data.path) {
            let laid_over = lay_over_the_file(
                files,
                &mine,
                settled,
                writes,
                saving,
                will_not_read,
                standoff,
            );

            let (brought, read_at) = match laid_over {
                Laid::Took { brought, read_at } => (brought, read_at),
                Laid::Raced => {
                    standoff.put_back(mine);
                    continue;
                }
                Laid::LeaveItAlone => {
                    standoff.put_back(mine);
                    standoff.hold();
                    return Err(error_stack::Report::new(StorageError::Flush)
                        .attach(StoreFileFact(files.data.path.clone()))
                        .attach(
                            "the file will not read and this store was told to leave one alone, \
                             so nothing was written: what is held is still held, and the next \
                             save tries again",
                        ));
                }
            };

            unmoved = read_at;

            for event in brought {
                if let Err(refused) = utils::emit_events(subscriptions, event) {
                    warn!(
                        file = %files.data.path.display(),
                        "an edit made outside was taken into this save and somebody could not \
                         read it back, and there is nobody to tell: the edit came from the \
                         file, not from a caller. {refused:?}"
                    );
                }
            }
        }

        match files.persist_while(unmoved) {
            Ok(Wrote::Replaced(standing)) => {
                persisted.store(saving, Ordering::Release);
                standoff.merged.store(laid, Ordering::Release);
                standoff.left_it(standing);
                return Ok(());
            }
            Ok(Wrote::FileMoved) => {
                standoff.put_back(mine);
                standoff.hold();
            }
            Err(why) => {
                standoff.put_back(mine);
                return Err(why);
            }
        }
    }

    Err(error_stack::Report::new(StorageError::Flush)
        .attach(StoreFileFact(files.data.path.clone()))
        .attach(
            "three times over, this save read the file and found the ground moved before it \
             could replace it - either somebody else wrote the file, or a write of ours \
             landed in the gap - so nothing was written",
        ))
}

/// Does what the store was told to do about a file it cannot parse.
///
/// The one place the three answers are spelled out, so a reader can see them
/// beside each other rather than inferring them from what the save does next.
fn what_to_do_about(
    file: &Path,
    rule: WhenItWillNotRead,
    why: error_stack::Report<StorageError>,
) -> Laid {
    match rule {
        // Answered before this is called: while the window stands it reads as
        // `Refuse`, and once it runs out as `SetAside`.
        WhenItWillNotRead::TryAgainFor(_) | WhenItWillNotRead::Refuse => {
            warn!(
                file = %file.display(),
                "the file will not read, so nothing was written: what this store holds stays \
                 in memory until the file parses again. {why:?}"
            );
            Laid::LeaveItAlone
        }

        WhenItWillNotRead::SetAside => {
            let aside = file.with_extension(match file.extension() {
                Some(had) => format!("{}.unreadable", had.to_string_lossy()),
                None => "unreadable".to_string(),
            });

            match std::fs::rename(file, &aside) {
                Ok(()) => {
                    warn!(
                        file = %file.display(),
                        aside = %aside.display(),
                        "the file will not read and was moved aside, so this save could go \
                         ahead: what was typed into it is still there under that name. {why:?}"
                    );
                    Laid::Took {
                        brought: Vec::new(),
                        read_at: None,
                    }
                }
                Err(io) => {
                    warn!(
                        file = %file.display(),
                        aside = %aside.display(),
                        error = %io,
                        "the file will not read and would not move aside either, so nothing \
                         was written rather than written over"
                    );
                    Laid::LeaveItAlone
                }
            }
        }

        WhenItWillNotRead::Overwrite => {
            warn!(
                file = %file.display(),
                "the file will not read, so this save writes the document whole and what was \
                 in the file is gone. {why:?}"
            );
            Laid::Took {
                brought: Vec::new(),
                read_at: None,
            }
        }
    }
}

/// The whole-name keys the file holds under `prefix`, as the document
/// addresses them.
///
/// A swept prefix comes off the file as an operation rather than as the list of
/// names this store happened to know: the tree half is one `delete_subtree`,
/// and this is the other half - the keys that live as whole names at the root
/// beside it. Read off the document being written, so a key another store put
/// there after the sweep was asked for goes with it, which is what happens to
/// the levels already.
///
/// The declarations are not needed to tell the two apart here. A tree's
/// outermost level spells one level, so it is only ever equal to a prefix and
/// never under one - and where it is equal, `delete_subtree` has taken it
/// already and this is a second delete of nothing.
fn plane_under<D: TextDocument>(doc: &D, prefix: &StorePath) -> Vec<StorePath> {
    let Some(root) = doc.get(&StorePath::root()) else {
        return Vec::new();
    };

    root.child_names()
        .into_iter()
        .filter(|name| {
            StorePath::parse_joined(name.as_str())
                .is_ok_and(|spelled| spelled.starts_with(prefix) && spelled != *prefix)
        })
        .map(StorePath::segment)
        .collect()
}

/// What came of laying this store's writes over what the file holds now.
enum Laid {
    /// The file was taken into the document. `brought` is what it carried in,
    /// and `read_at` is how it stood when it was read - the version the save
    /// that follows has to still find there.
    Took {
        brought: Vec<StoreEvent>,
        read_at: Option<(u64, std::time::SystemTime)>,
    },

    /// A write of ours landed between the file being read and the document
    /// lock being taken, so it is in the document and not in what this save is
    /// holding. Laying that over the file would drop it. Reading again settles
    /// it, the way it does for [`super::watching::look`].
    Raced,

    /// The file is not to be touched at all, and what this store holds stays
    /// where it is - see [`WhenItWillNotRead::Refuse`].
    LeaveItAlone,
}

/// Puts what this store wrote back over what the file holds now, and says what
/// the file brought with it and how it stood when it was read.
///
/// `saving` is `writes` as it stood before this save took what it is holding,
/// so a document that has moved past it holds a write this save was not given.
/// The file is read before the lock - it has to be, the read is the slow part -
/// and that is the gap the check closes.
fn lay_over_the_file<D: TextDocument>(
    files: &StoreFiles<D>,
    mine: &Touched,
    settled: &AtomicU64,
    writes: &AtomicU64,
    saving: u64,
    will_not_read: WhenItWillNotRead,
    standoff: &Standoff,
) -> Laid {
    let refuse = |why: &str| {
        warn!(
            file = %files.data.path.display(),
            "the file was edited outside while this store held writes of its own, and {why}, \
             so this save writes the document whole and what was in the file is gone"
        );
        Laid::Took {
            brought: Vec::new(),
            read_at: None,
        }
    };

    let read_at = standing_of(&files.data.path);

    let on_disk = match files.data.load_or_empty() {
        Ok(on_disk) => {
            standoff.reads_again();
            on_disk
        }
        Err(why) => {
            let rule = match will_not_read {
                WhenItWillNotRead::TryAgainFor(window) if standoff.unreadable_for() < window => {
                    WhenItWillNotRead::Refuse
                }
                WhenItWillNotRead::TryAgainFor(_) => WhenItWillNotRead::SetAside,
                settled => settled,
            };

            return what_to_do_about(&files.data.path, rule, why);
        }
    };

    let mut guard = files.data.doc.write();

    if writes.load(Ordering::Acquire) != saving {
        return Laid::Raced;
    }

    if has_no_keys(&on_disk) && !has_no_keys(&*guard) {
        return refuse("it came back holding nothing where this store holds keys");
    }

    let before = guard.clone();
    let mut merged = on_disk;

    for under in &mine.under {
        if let Err(why) = merged.delete_subtree(under) {
            return refuse(&format!(
                "a level this store swept would not come off it: {why:?}"
            ));
        }

        for beside in plane_under(&merged, under) {
            if let Err(why) = merged.delete(&beside) {
                return refuse(&format!(
                    "a key beside a level this store swept would not come off it: {why:?}"
                ));
            }
        }
    }

    for at in &mine.at {
        let laid = match guard.get(at) {
            Some(node) => merged.set(at, node.clone()),
            None => merged.delete(at).map(|_| ()),
        };

        if let Err(why) = laid {
            return refuse(&format!(
                "a place this store wrote would not go back on it: {why:?}"
            ));
        }
    }

    *guard = merged;

    let at = settled.fetch_add(1, Ordering::AcqRel) + 1;
    let brought = match diff_documents::<D>(&before, &guard, at) {
        Ok(events) => events,
        Err(why) => {
            warn!(
                file = %files.data.path.display(),
                "an edit made outside was taken into this save and could not be read, so \
                 nobody was told about it: {why:?}"
            );
            Vec::new()
        }
    };

    Laid::Took { brought, read_at }
}
