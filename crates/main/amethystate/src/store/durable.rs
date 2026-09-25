use crate::store::error::{StorageError, StorageResult};
use error_stack::{AttachmentKind, FrameKind, Report};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};

/// A view over a primitive whose writes return only once they are on disk.
///
/// Every write the primitive offers has a form here, so the guarantee never
/// costs a second call the caller could be preempted between - or forget.
///
/// **What lands beside it is the engine's answer.** redb and sqlite commit the
/// write that was asked for and leave the rest of the buffer where it is; a
/// document engine keeps the store in one file and rewrites the whole of it to
/// save any of it, so one durable write makes every buffered value durable with
/// it. Neither is a gap - the file is rewritten either way - and the difference
/// shows in exactly one place, which is what a crash leaves behind.
///
/// [`Backend::a_commit_covers_the_whole_store`](crate::store::builder::Backend::a_commit_covers_the_whole_store)
/// is the answer said out loud, for an application that has to know.
pub struct Durable<'a, T>(pub(crate) &'a T);

/// Announces how flushes ended, to whoever is waiting on one.
///
/// A flush takes a number as it begins, before it gathers what it writes, and
/// a waiter notes the last number taken when it starts waiting. Only a flush
/// numbered above that can answer it: one already under way may have gathered
/// its writes before the waiter's own was there.
#[derive(Default)]
pub struct CommitSignal {
    begun: AtomicU64,
    settled: Mutex<Settled>,
    waiters: Mutex<Vec<Waker>>,
}

/// The highest-numbered flush that landed, the highest that failed with why it
/// failed, the failure last given up on with the flush it reaches to, and
/// whether the store has closed, after which no flush begins.
#[derive(Default)]
struct Settled {
    landed: u64,
    failed: u64,
    last_failure: Option<(u64, Arc<Report<StorageError>>)>,
    given_up: Option<(u64, Arc<Report<StorageError>>)>,
    closed: bool,
}

impl CommitSignal {
    /// Numbers a flush that is about to gather what it writes.
    pub(crate) fn begin(&self) -> u64 {
        self.begun.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Records how the flush numbered `flush` ended.
    ///
    /// A landing answers whoever waits. A failure answers only a waiter that
    /// asked for one attempt: the flush is retried, and the other waiters hear
    /// of it once it is given up on.
    pub(crate) fn settle(&self, flush: u64, ended: &StorageResult<()>) {
        let mut settled = self.settled.lock().unwrap();

        match ended {
            Ok(()) => settled.landed = settled.landed.max(flush),
            Err(why) => {
                settled.failed = settled.failed.max(flush);
                if settled
                    .last_failure
                    .as_ref()
                    .is_none_or(|(at, _)| *at < flush)
                {
                    settled.last_failure = Some((flush, Arc::new(copied(why))));
                }
            }
        }

        drop(settled);
        self.wake();
    }

    /// Answers whoever waits on a flush that failed, with why.
    pub(crate) fn gave_up(&self, why: Arc<Report<StorageError>>) {
        {
            let mut settled = self.settled.lock().unwrap();
            let through = settled.failed;
            settled.given_up = Some((through, why));
        }
        self.wake();
    }

    /// Answers whoever still waits that the store closed, once its last flush
    /// has settled: nothing begins after this, so a flush they are waiting for
    /// never will.
    pub(crate) fn closed(&self) {
        self.settled.lock().unwrap().closed = true;
        self.wake();
    }

    fn begun(&self) -> u64 {
        self.begun.load(Ordering::Acquire)
    }

    fn answer(&self, after: u64, attempt: bool) -> Option<StorageResult<()>> {
        let settled = self.settled.lock().unwrap();

        if settled.landed > after {
            return Some(Ok(()));
        }

        if attempt
            && let Some((at, why)) = &settled.last_failure
            && *at > after
        {
            return Some(Err(copied(why)));
        }

        match &settled.given_up {
            Some((through, why)) if *through > after => Some(Err(commit_failed(why))),
            _ if settled.closed => Some(Err(Report::new(StorageError::Closed))),
            _ => None,
        }
    }

    fn park(&self, cx: &Context<'_>) {
        self.waiters.lock().unwrap().push(cx.waker().clone());
    }

    fn wake(&self) {
        let woken = std::mem::take(&mut *self.waiters.lock().unwrap());
        for waker in woken {
            waker.wake();
        }
    }
}

/// Whether the store can still persist, and why it cannot.
///
/// Set by the background flush once a failing streak outlives its retry
/// budget and the application asked to be failed rather than ignored, and
/// cleared by the next flush that lands - so a full disk that gets emptied
/// heals the store without a restart. Reads never consult it: what is on
/// disk is still readable, and what is buffered is still buffered.
///
/// It also keeps the last failure whatever was decided about it, and the
/// observers that hear both the failure and the save that ends it.
#[derive(Default)]
pub struct PersistHealth {
    given_up: Mutex<Option<Arc<Report<StorageError>>>>,
    last: Mutex<Option<Arc<Report<StorageError>>>>,
    observers: Mutex<Vec<(u64, crate::store::config::PersistObserver)>>,
    next_observer: AtomicU64,
}

/// Keeps an observer attached to a store's background saving; dropping it
/// detaches the observer.
#[must_use = "dropped here, the observer is detached here"]
pub struct PersistWatch {
    health: Weak<PersistHealth>,
    id: u64,
}

impl PersistWatch {
    pub(crate) fn detached() -> Self {
        Self {
            health: Weak::new(),
            id: 0,
        }
    }
}

impl Drop for PersistWatch {
    fn drop(&mut self) {
        if let Some(health) = self.health.upgrade() {
            health
                .observers
                .lock()
                .unwrap()
                .retain(|(id, _)| *id != self.id);
        }
    }
}

impl PersistHealth {
    /// The failure the last failing streak gave up with, until a save lands
    /// again - whatever was decided about it.
    pub fn last_failure(&self) -> Option<Arc<Report<StorageError>>> {
        self.last.lock().unwrap().clone()
    }

    /// Attaches an observer, which hears every streak that gives up and the
    /// save that lands after one.
    pub fn observe(
        self: &Arc<Self>,
        observer: crate::store::config::PersistObserver,
    ) -> PersistWatch {
        let id = self.next_observer.fetch_add(1, Ordering::Relaxed) + 1;
        self.observers.lock().unwrap().push((id, observer));
        PersistWatch {
            health: Arc::downgrade(self),
            id,
        }
    }

    pub(crate) fn observers(&self) -> Vec<crate::store::config::PersistObserver> {
        self.observers
            .lock()
            .unwrap()
            .iter()
            .map(|(_, observer)| observer.clone())
            .collect()
    }

    pub(crate) fn failed(&self, reason: Arc<Report<StorageError>>) {
        *self.last.lock().unwrap() = Some(reason);
    }

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

    pub(crate) fn landed(&self) -> bool {
        *self.given_up.lock().unwrap() = None;
        self.last.lock().unwrap().take().is_some()
    }
}

/// A write the store is making, to be awaited.
///
/// What it waits for is said by whatever handed it out: a flush that lands,
/// one attempt at one, or a close. It holds no thread and needs no particular
/// runtime - any executor can await it.
pub struct Commit(Awaiting);

enum Awaiting {
    Flush {
        signal: Arc<CommitSignal>,
        after: u64,
        attempt: bool,
    },
    Handed(Arc<Handoff>),
    Gone,
}

/// Where work done elsewhere leaves its answer for the commit awaiting it.
#[derive(Default)]
pub(crate) struct Handoff {
    outcome: Mutex<Option<StorageResult<()>>>,
    waker: Mutex<Option<Waker>>,
}

impl Handoff {
    pub(crate) fn finish(&self, outcome: StorageResult<()>) {
        *self.outcome.lock().unwrap() = Some(outcome);
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }

    fn poll(&self, cx: &Context<'_>) -> Poll<StorageResult<()>> {
        if let Some(outcome) = self.outcome.lock().unwrap().take() {
            return Poll::Ready(outcome);
        }

        *self.waker.lock().unwrap() = Some(cx.waker().clone());

        match self.outcome.lock().unwrap().take() {
            Some(outcome) => Poll::Ready(outcome),
            None => Poll::Pending,
        }
    }
}

impl Commit {
    /// Resolves once a flush that began after it has landed, or once the
    /// failing streak that flush joined is given up on.
    pub(crate) fn awaiting(signal: Arc<CommitSignal>) -> Self {
        let after = signal.begun();
        Self(Awaiting::Flush {
            signal,
            after,
            attempt: false,
        })
    }

    /// Resolves with how the first flush that began after it ended - landed,
    /// or failed and why - without waiting out the retries a failure starts.
    pub(crate) fn attempt(signal: Arc<CommitSignal>) -> Self {
        let after = signal.begun();
        Self(Awaiting::Flush {
            signal,
            after,
            attempt: true,
        })
    }

    /// Resolves with what the other end of `handoff` finishes with.
    pub(crate) fn handed(handoff: Arc<Handoff>) -> Self {
        Self(Awaiting::Handed(handoff))
    }

    /// Answered at once, with `outcome`.
    pub(crate) fn ready(outcome: StorageResult<()>) -> Self {
        let handoff = Arc::new(Handoff::default());
        handoff.finish(outcome);
        Self::handed(handoff)
    }

    /// Runs `work` on a thread of its own and resolves with what it returns,
    /// so a call that blocks on the disk is awaited without holding an
    /// executor's thread while it does.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn on_a_thread(work: impl FnOnce() -> StorageResult<()> + Send + 'static) -> Self {
        let handoff = Arc::new(Handoff::default());
        let done = handoff.clone();

        let spawned = std::thread::Builder::new()
            .name("amethystate-close".into())
            .spawn(move || done.finish(work()));

        match spawned {
            Ok(_) => Self::handed(handoff),
            Err(io) => Self::ready(Err(Report::new(StorageError::Flush).attach(format!(
                "no thread could be started to wait on the disk: {io}"
            )))),
        }
    }

    /// A commit for a store that is no longer there, answered at once: the
    /// store was closed.
    pub(crate) fn gone() -> Self {
        Self(Awaiting::Gone)
    }
}

impl Future for Commit {
    type Output = StorageResult<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (signal, after, attempt) = match &self.0 {
            Awaiting::Flush {
                signal,
                after,
                attempt,
            } => (signal, *after, *attempt),
            Awaiting::Handed(handoff) => return handoff.poll(cx),
            Awaiting::Gone => return Poll::Ready(Err(Report::new(StorageError::Closed))),
        };

        if let Some(answer) = signal.answer(after, attempt) {
            return Poll::Ready(answer);
        }

        signal.park(cx);

        match signal.answer(after, attempt) {
            Some(answer) => Poll::Ready(answer),
            None => Poll::Pending,
        }
    }
}

/// The same failure again, for another waiter: a `Report` is not `Clone`, so
/// its context is kept and what it said is carried across as text.
fn copied(why: &Report<StorageError>) -> Report<StorageError> {
    let mut outermost = true;

    why.frames().fold(
        Report::new(*why.current_context()),
        |told, frame| match frame.kind() {
            FrameKind::Context(_) if outermost => {
                outermost = false;
                told
            }
            FrameKind::Context(context) => told.attach(context.to_string()),
            FrameKind::Attachment(AttachmentKind::Printable(said)) => told.attach(said.to_string()),
            FrameKind::Attachment(_) => told,
        },
    )
}

/// What a waiter is told when its flush was given up on: that the commit did
/// not happen, carrying everything the flush said about why.
fn commit_failed(why: &Report<StorageError>) -> Report<StorageError> {
    why.frames().fold(
        Report::new(StorageError::CommitFailed),
        |told, frame| match frame.kind() {
            FrameKind::Context(context) => told.attach(context.to_string()),
            FrameKind::Attachment(AttachmentKind::Printable(said)) => told.attach(said.to_string()),
            FrameKind::Attachment(_) => told,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(commit: &mut Commit) -> Poll<StorageResult<()>> {
        Pin::new(commit).poll(&mut Context::from_waker(Waker::noop()))
    }

    fn said(why: &Report<StorageError>) -> String {
        let attached: Vec<&str> = why
            .frames()
            .filter_map(|frame| frame.downcast_ref::<String>().map(String::as_str))
            .collect();

        format!("{}\n{}", why.current_context(), attached.join("\n"))
    }

    #[test]
    fn a_flush_already_under_way_does_not_answer_a_waiter_that_came_after_it() {
        let signal = Arc::new(CommitSignal::default());
        let under_way = signal.begin();
        let mut commit = Commit::awaiting(signal.clone());

        signal.settle(under_way, &Ok(()));

        assert!(
            poll(&mut commit).is_pending(),
            "that flush gathered its writes before this waiter's write was there"
        );

        let next = signal.begin();
        signal.settle(next, &Ok(()));

        assert!(matches!(poll(&mut commit), Poll::Ready(Ok(()))));
    }

    #[test]
    fn a_waiter_hears_why_the_flush_it_waited_on_gave_up() {
        let signal = Arc::new(CommitSignal::default());
        let mut commit = Commit::awaiting(signal.clone());

        let attempt = signal.begin();
        signal.settle(
            attempt,
            &Err(Report::new(StorageError::Flush).attach("the disk is full")),
        );

        assert!(
            poll(&mut commit).is_pending(),
            "an attempt inside the retry budget answers nobody"
        );

        signal.gave_up(Arc::new(
            Report::new(StorageError::Flush).attach("the disk is full"),
        ));

        let Poll::Ready(Err(why)) = poll(&mut commit) else {
            panic!("a waiter was not told the flush gave up");
        };

        insta::assert_snapshot!(said(&why));
    }

    #[test]
    fn a_landing_after_the_give_up_answers_a_waiter_that_had_not_heard_yet() {
        let signal = Arc::new(CommitSignal::default());
        let mut commit = Commit::awaiting(signal.clone());

        let failed = signal.begin();
        signal.settle(failed, &Err(Report::new(StorageError::Flush)));
        signal.gave_up(Arc::new(Report::new(StorageError::Flush)));

        let landed = signal.begin();
        signal.settle(landed, &Ok(()));

        assert!(matches!(poll(&mut commit), Poll::Ready(Ok(()))));
    }

    #[test]
    fn a_waiter_no_flush_will_answer_hears_that_the_store_closed() {
        let signal = Arc::new(CommitSignal::default());
        let mut commit = Commit::awaiting(signal.clone());

        assert!(poll(&mut commit).is_pending());
        signal.closed();

        let Poll::Ready(Err(why)) = poll(&mut commit) else {
            panic!("a waiter went on waiting for a flush a closed store will never begin");
        };
        assert_eq!(*why.current_context(), StorageError::Closed);
    }

    #[test]
    fn a_flush_that_landed_before_the_close_still_answers_as_landed() {
        let signal = Arc::new(CommitSignal::default());
        let mut commit = Commit::awaiting(signal.clone());

        let last = signal.begin();
        signal.settle(last, &Ok(()));
        signal.closed();

        assert!(matches!(poll(&mut commit), Poll::Ready(Ok(()))));
    }

    #[test]
    fn a_commit_for_a_store_that_is_gone_says_it_was_closed() {
        let Poll::Ready(Err(why)) = poll(&mut Commit::gone()) else {
            panic!("a commit for a store that is gone did not answer at once");
        };

        assert_eq!(*why.current_context(), StorageError::Closed);
    }
}
