use super::document::TextDocument;
use super::files::{StoreFile, has_no_keys};
use super::store::diff_documents;
use crate::store::StoreEvent;
use crate::store::SubscriptionEntry;
use crate::store::backend::utils;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Where the document stands against the file, for the one question that
/// matters when an outside edit arrives: may it be taken.
///
/// Two counters answer it, and neither answers it alone. `writes` rises with
/// every mutation while the document lock is held; `persisted` rises to meet it
/// when a flush lands. The third number is the one read *before* the file was,
/// which is what separates "we have unsaved work" from "a flush landed while we
/// were reading".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Standing {
    /// Document and file agree. What the file says may replace what is held.
    Settled,

    /// The document holds writes the file was never given. The file is behind,
    /// and what it says is not an edit but an older copy of our own work.
    Unsaved,

    /// A flush landed between reading the file and taking the lock, so what
    /// came back is a version behind. Reading again settles it.
    Raced,
}

pub(super) fn standing(writes: &AtomicU64, persisted: &AtomicU64, read_after: u64) -> Standing {
    let written = writes.load(Ordering::Acquire);
    let saved = persisted.load(Ordering::Acquire);

    if written != saved {
        Standing::Unsaved
    } else if saved != read_after {
        Standing::Raced
    } else {
        Standing::Settled
    }
}

/// What came of one look at the file.
#[derive(Debug)]
pub(super) enum Taken {
    /// The document now holds what the file said, and these are the changes.
    Applied(Vec<StoreEvent>),

    /// The file says what the document already says.
    Same,

    /// Not taken, and why.
    Held(Standing),

    /// The file could not be read or would not parse. A half-written file
    /// looks like this, and the write that finishes it brings another event.
    Unreadable,
}

/// One look: read the file, decide whether it may be taken, and take it.
///
/// The file is read before the lock, so what came back can be a version behind
/// by the time the decision is made. That is what [`Standing::Raced`] is, and
/// it is the caller's to retry - dropping it would throw away somebody's edit
/// because a flush of ours happened to land in the gap.
pub(super) fn look<D: TextDocument>(
    file: &StoreFile<D>,
    writes: &AtomicU64,
    persisted: &AtomicU64,
    settled: &AtomicU64,
) -> Taken {
    let read_after = persisted.load(Ordering::Acquire);

    let Ok(content) = std::fs::read_to_string(&file.path) else {
        return Taken::Unreadable;
    };

    // A watcher wakes on the file being touched, and touched is not changed.
    // Bytes identical to the ones this store wrote are content identical, so
    // there is nothing here to take - and finding that out this way costs one
    // pass over what was read anyway, where the reading below costs a parse of
    // the whole file and the comparison after it costs two renders.
    if file.wrote_exactly(&content) {
        return Taken::Same;
    }

    let Ok(on_disk) = D::parse(&content) else {
        return Taken::Unreadable;
    };

    let mut guard = file.doc.write();

    match standing(writes, persisted, read_after) {
        Standing::Settled => {}
        held => return Taken::Held(held),
    }

    // A format that calls an empty file a valid empty document hands one back
    // without complaint, and taking it would read a file caught half-written as
    // every key being deleted. Here there is no need to ask the bookkeeping:
    // what is held answers it, and a store somebody emptied through the API
    // emptied this too.
    if has_no_keys(&on_disk) && !has_no_keys(&*guard) {
        warn!(
            file = %file.path.display(),
            "the file came back holding nothing where the store holds keys, so it was left \
             alone: a file is read as empty only when the store agrees it is"
        );
        return Taken::Unreadable;
    }

    let held = guard.serialize().unwrap_or_default();
    let found = on_disk.serialize().unwrap_or_default();
    if held == found {
        return Taken::Same;
    }

    let before = guard.clone();
    *guard = on_disk;
    file.agrees_with(&content);
    info!("external store change detected");

    let at = settled.fetch_add(1, Ordering::AcqRel) + 1;

    match diff_documents::<D>(&before, &guard, at) {
        Ok(events) => Taken::Applied(events),
        Err(e) => {
            warn!("an external edit could not be read, so nobody was told about it: {e:?}");
            Taken::Applied(Vec::new())
        }
    }
}

/// How many times a look that lost a race is worth taking again.
///
/// A race needs a flush to land in the gap between reading the file and taking
/// the lock. Two in a row means writes are arriving faster than the file can be
/// read, and the next file event will bring the edit round again anyway.
const RETRIES: usize = 3;

/// Takes what the file says, retrying a look that lost a race, and tells
/// whoever is subscribed.
///
/// Runs on the thread that noticed the file change - which is the thread the
/// change came from, the same rule a write follows. Nothing here is joined by
/// [`close`](super::store::TextStoreInner::close), so a subscriber may close the
/// store from inside its own callback.
pub(super) fn take_outside_edit<D: TextDocument>(
    file: &StoreFile<D>,
    subscriptions: &RwLock<Vec<SubscriptionEntry>>,
    writes: &AtomicU64,
    persisted: &AtomicU64,
    standoff: &super::standoff::Standoff,
    settled: &AtomicU64,
) {
    for _ in 0..RETRIES {
        match look(file, writes, persisted, settled) {
            Taken::Applied(events) => {
                for event in events {
                    if let Err(refused) = utils::emit_events(subscriptions, event) {
                        warn!(
                            file = %file.path.display(),
                            "an outside edit was taken and somebody could not read it back, and \
                             there is nobody to tell: the edit came from the file, not from a \
                             caller. {refused:?}"
                        );
                    }
                }
                return;
            }
            Taken::Same => return,
            Taken::Unreadable => {
                standoff.hold();
                return;
            }
            Taken::Held(Standing::Raced) => continue,
            Taken::Held(Standing::Unsaved) => {
                standoff.hold();
                info!(
                    file = %file.path.display(),
                    "the file was edited outside while this store held writes it had not saved, \
                     so the edit stays in the file: the next save lays this store's own paths \
                     over what is there rather than replacing it"
                );
                return;
            }
            Taken::Held(Standing::Settled) => return,
        }
    }

    warn!(
        file = %file.path.display(),
        "an outside edit lost the race with this store's own saving three times over, so it was \
         not taken; the next change to the file brings it round again"
    );
}

/// Waits out a quiet period before looking, so a burst of file events becomes
/// one look rather than one each.
///
/// The notifier calls its handler one at a time, so waiting here holds the next
/// event rather than losing it. A look that happens anyway costs a file read
/// and finds nothing: what is emitted comes from comparing documents, not from
/// counting events.
/// The wait ends when the store goes, however long the quiet period is. It has
/// to: a store's own save is a change to the file, so the store wakes its own
/// watcher, and a wait that only ran out on the clock held the notifier's
/// thread - and the handle it watches the file with - for the whole period
/// after the store it belongs to was dropped. Opening stores faster than that
/// piles them up until the process runs out of descriptors.
pub(super) struct Coalescing {
    quiet: Duration,
    waiting: Mutex<Waiting>,
    woken: Condvar,
}

struct Waiting {
    until: Instant,
    stopped: bool,
}

impl Coalescing {
    pub(super) fn new(quiet: Duration) -> Arc<Self> {
        Arc::new(Self {
            quiet,
            waiting: Mutex::new(Waiting {
                until: Instant::now(),
                stopped: false,
            }),
            woken: Condvar::new(),
        })
    }

    /// Ends the wait in progress and refuses the ones after it.
    pub(super) fn stop(&self) {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        waiting.stopped = true;
        self.woken.notify_all();
    }

    /// Blocks until the file has been quiet for the period this was built with.
    ///
    /// `false` when [`Coalescing::stop`] ended the wait instead: the store this
    /// belongs to is going, and what the file holds is no longer its business.
    pub(super) fn settle(&self) -> bool {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());

        if waiting.stopped {
            return false;
        }
        waiting.until = Instant::now() + self.quiet;

        loop {
            let now = Instant::now();
            if waiting.stopped {
                return false;
            }
            if now >= waiting.until {
                return true;
            }

            let left = waiting.until - now;
            waiting = self
                .woken
                .wait_timeout(waiting, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Coalescing;
    use std::time::{Duration, Instant};

    #[test]
    fn a_wait_ends_when_the_store_it_belongs_to_does() {
        let settling = Coalescing::new(Duration::from_secs(60));
        let stopper = settling.clone();

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            stopper.stop();
        });

        let started = Instant::now();
        let settled = settling.settle();

        assert!(!settled, "a stopped wait is not a settled one");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the wait ran out the whole quiet period after it was stopped: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_wait_asked_for_after_the_stop_does_not_begin() {
        let settling = Coalescing::new(Duration::from_secs(60));
        settling.stop();

        let started = Instant::now();

        assert!(!settling.settle());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_wait_nobody_stopped_runs_out_the_quiet_period() {
        let settling = Coalescing::new(Duration::from_millis(200));

        let started = Instant::now();

        assert!(settling.settle(), "nobody stopped it");
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "it came back early: {:?}",
            started.elapsed()
        );
    }
}
