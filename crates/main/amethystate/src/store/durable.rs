use crate::store::error::{StorageError, StorageResult};
use error_stack::Report;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// A view over a primitive whose writes return only once they are on disk.
///
/// Every write the primitive offers has a form here, so the guarantee never
/// costs a second call the caller could be preempted between - or forget.
pub struct Durable<'a, T>(pub(crate) &'a T);

/// Announces that a flush finished, to whoever is waiting on one.
///
/// Waiters key off a generation counter, so several of them riding on one flush
/// all resolve from that single commit.
#[derive(Default)]
pub struct CommitSignal {
    generation: AtomicU64,
    last_failed: AtomicBool,
    waiters: Mutex<Vec<Waker>>,
}

impl CommitSignal {
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Called after every flush attempt, successful or not. A failed attempt
    /// still has to wake its waiters, or they would hang on a store that never
    /// manages to write.
    pub(crate) fn finished(&self, ok: bool) {
        self.last_failed.store(!ok, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);

        let woken = std::mem::take(&mut *self.waiters.lock().unwrap());
        for waker in woken {
            waker.wake();
        }
    }

    fn failed(&self) -> bool {
        self.last_failed.load(Ordering::Acquire)
    }

    fn park(&self, cx: &Context<'_>) {
        self.waiters.lock().unwrap().push(cx.waker().clone());
    }
}

/// Whether the store can still persist, and why it cannot.
///
/// Set by the background flush once a failing streak outlives its retry
/// budget and the application asked to be failed rather than ignored, and
/// cleared by the next flush that lands - so a full disk that gets emptied
/// heals the store without a restart. Reads never consult it: what is on
/// disk is still readable, and what is buffered is still buffered.
#[derive(Default)]
pub struct PersistHealth {
    given_up: Mutex<Option<Arc<Report<StorageError>>>>,
}

impl PersistHealth {
    /// Why writes are failing, if they are.
    ///
    /// The failure itself: a caller deciding what to do about it reads
    /// `current_context()`, and one reporting it renders with `{:#}`. Behind an
    /// `Arc` because a `Report` is not `Clone` and this is read by every writer
    /// while the streak lasts.
    pub fn failure(&self) -> Option<Arc<Report<StorageError>>> {
        self.given_up.lock().unwrap().clone()
    }

    pub(crate) fn give_up(&self, reason: Arc<Report<StorageError>>) {
        *self.given_up.lock().unwrap() = Some(reason);
    }

    pub(crate) fn landed(&self) {
        *self.given_up.lock().unwrap() = None;
    }
}

/// Resolves once a flush has completed since it was created.
pub struct Commit {
    signal: Arc<CommitSignal>,
    start: u64,
}

impl Commit {
    pub(crate) fn awaiting(signal: Arc<CommitSignal>) -> Self {
        let start = signal.generation();
        Self { signal, start }
    }

    /// A commit for a store that is no longer there: already finished, and
    /// finished as a failure, so an awaiting caller is told at once.
    pub(crate) fn gone() -> Self {
        let signal = Arc::new(CommitSignal::default());
        signal.finished(false);
        Self { signal, start: 0 }
    }
}

impl Future for Commit {
    type Output = StorageResult<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let signal = &self.signal;

        if signal.generation() > self.start {
            return Poll::Ready(outcome(signal));
        }

        signal.park(cx);

        if signal.generation() > self.start {
            Poll::Ready(outcome(signal))
        } else {
            Poll::Pending
        }
    }
}

fn outcome(signal: &CommitSignal) -> StorageResult<()> {
    if signal.failed() {
        Err(error_stack::Report::new(StorageError::CommitFailed))
    } else {
        Ok(())
    }
}
