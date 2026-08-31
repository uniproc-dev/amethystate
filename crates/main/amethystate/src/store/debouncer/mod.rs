//! One thread that waits out a quiet period and then writes.
//!
//! Where the thread can be:
//!
//! - **Idle** - parked on `recv`, nothing owed.
//! - **Settling** - a `Schedule` arrived; waiting out `interval`, and each
//!   further `Schedule` starts the wait again.
//! - **Flushing** - inside `op`.
//! - **Retrying** - `op` failed; waiting `retry.interval` for the next
//!   attempt, and reading the channel while it waits.
//! - **Silent** - the same, past `retry.budget`: the streak has been reported
//!   once and the attempts carry on without saying more.
//! - **Stopped** - the thread has left. `schedule` and `flush_now` do nothing.
//! - **Poisoned** - `op` panicked and took the thread down holding `guard`.
//!   Every later `schedule` panics too.
//!
//! | from | `Schedule` | `Now` | `Stop` | timeout | `op` ok | `op` err |
//! |---|---|---|---|---|---|---|
//! | Idle | Settling | Flushing | Stopped | - | - | - |
//! | Settling | Settling | Flushing | Flushing, then Stopped | Flushing | - | - |
//! | Flushing | - | - | - | - | Idle | Retrying |
//! | Retrying | Flushing | Flushing | Stopped | Flushing | - | - |
//! | Silent | Flushing | Flushing | Stopped | Flushing | - | - |
//! | Stopped | - | - | - | - | - | - |
//!
//! A disconnected channel reads as `Stop` everywhere it is noticed, and a
//! panic inside `op` moves to Poisoned from Flushing.
//!
//! The asymmetry in the `Stop` column is the one thing to hold on to: stopping
//! during a quiet period runs the write that period was waiting out, and
//! stopping during a failing streak does not. A settling write has not been
//! tried yet; a failing one has, and another attempt would only hold whoever
//! asked to stop.
//!
//! `Stop` is not read inside `op`, so a caller waiting on the thread waits for
//! at most one flush and one retry interval.

use crate::store::StorageResult;
use crate::store::config::{AfterGivingUp, PersistFailureCallback, RetryPolicy};
use crate::store::durable::{CommitSignal, PersistHealth};
use crate::store::util::DeadNotifier;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, error, warn};

mod machine;

use machine::{Next, State, Trigger, next_state};

pub struct Debouncer {
    tx: mpsc::Sender<Trigger>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    stopped: AtomicBool,
    guard: Arc<Mutex<()>>,
    #[cfg(test)]
    dead: Arc<(Mutex<bool>, Condvar)>,
}

/// Everything a retrying flush needs beyond the work itself: how often to try
/// again, who to wake, where to record that it is not working, and who to ask
/// what that should mean.
pub struct FlushPolicy {
    pub retry: RetryPolicy,
    pub commits: Arc<CommitSignal>,
    pub health: Arc<PersistHealth>,
    pub on_giveup: Option<PersistFailureCallback>,
}

impl Debouncer {
    fn spawn<F>(interval: Duration, mut run: F) -> Self
    where
        F: FnMut(&mpsc::Receiver<Trigger>, bool) -> Next + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<Trigger>();
        let guard = Arc::new(Mutex::new(()));
        let dead = Arc::new((Mutex::new(false), Condvar::new()));
        let guard_inner = guard.clone();
        let dead_inner = dead.clone();

        let handle = thread::spawn(move || {
            let _notify = DeadNotifier(dead_inner);
            let _hold = guard_inner.lock().unwrap();

            let mut state = State::Idle;
            while state != State::Stopped {
                state = next_state(state, &rx, interval, &mut run);
            }

            debug!("debouncer thread exiting");
        });

        Self {
            tx,
            handle: Mutex::new(Some(handle)),
            stopped: AtomicBool::new(false),
            guard,
            #[cfg(test)]
            dead,
        }
    }

    pub fn new<F>(interval: Duration, mut op: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        Self::spawn(interval, move |_, _| {
            op();
            Next::Wake
        })
    }

    /// Like [`Debouncer::new`], for a flush that has to report whether it
    /// landed rather than just running.
    ///
    /// `op` returns `Ok(())` once the buffered writes are on disk, or the
    /// reason they are not. A failure is retried at `policy.retry.interval`
    /// and keeps being retried until it lands or the store is dropped - a
    /// full disk is usually temporary, and a store that stopped trying could
    /// not heal when it was fixed. What `policy.retry.budget` bounds is the
    /// silence: a streak outliving it escalates once, waking anyone awaiting
    /// that flush with a failure and asking `policy.on_giveup` what writers
    /// should be told from here.
    pub fn new_with_retry<F>(interval: Duration, policy: FlushPolicy, mut op: F) -> Self
    where
        F: FnMut() -> StorageResult<()> + Send + 'static,
    {
        Self::spawn(interval, move |rx, last| {
            run_with_retry(&mut op, &policy, rx, last)
        })
    }

    pub fn schedule(&self) {
        self.send(Trigger::Schedule);
    }

    /// Runs the operation without waiting out the quiet period.
    pub fn flush_now(&self) {
        self.send(Trigger::Now);
    }

    fn send(&self, trigger: Trigger) {
        if self.guard.is_poisoned() {
            panic!("debouncer is poisoned");
        }
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if let Err(e) = self.tx.send(trigger)
            && !self.stopped.load(Ordering::Acquire)
        {
            panic!("failed to schedule debounced operation: channel closed ({e})");
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.guard.is_poisoned()
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Refuses further work, without waiting for the thread.
    ///
    /// Split from [`Debouncer::shutdown`] so a caller can take this decision
    /// while holding whatever lock its writers buffer under: waiting for the
    /// thread there would deadlock, since the flush the thread is running
    /// wants that same lock.
    ///
    /// Answers whether this call was the one that stopped it.
    pub fn stop_accepting(&self) -> bool {
        !self.stopped.swap(true, Ordering::AcqRel)
    }

    /// Ends the thread and waits for it, so anything already queued has run by
    /// the time this returns.
    ///
    /// Calling it more than once is fine; the second caller finds the thread
    /// already taken and does not wait again.
    pub fn shutdown(&self) {
        self.stop_accepting();
        let _ = self.tx.send(Trigger::Stop);

        let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
impl Debouncer {
    pub fn wait_dead(&self) {
        let (lock, cvar) = &*self.dead;
        let _unused = cvar.wait_while(lock.lock().unwrap(), |dead| !*dead);
    }
}

impl Drop for Debouncer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

enum Streak {
    Fresh,
    Failing { since: Instant },
    GaveUp,
}

fn give_up(
    reason: &Arc<error_stack::Report<crate::store::StorageError>>,
    elapsed: Duration,
    policy: &FlushPolicy,
) {
    policy.commits.finished(false);

    let decision = policy
        .on_giveup
        .as_ref()
        .map_or(AfterGivingUp::Fail, |callback| callback(reason));

    error!(
        target: "amethystate",
        reason = %format!("{reason:#}"),
        kind = ?reason.current_context(),
        elapsed_ms = elapsed.as_millis() as u64,
        budget_ms = policy.retry.budget.as_millis() as u64,
        decision = ?decision,
        "background flush has been failing longer than its retry budget",
    );

    match decision {
        AfterGivingUp::Fail => policy.health.give_up(reason.clone()),
        AfterGivingUp::Ignore => {}
        AfterGivingUp::Poison => panic!(
            "background flush failed for {elapsed:?} (budget {:?}): {reason:#}",
            policy.retry.budget
        ),
    }
}

/// Runs `op` until it lands or the store goes away, retrying at
/// `policy.retry.interval`.
///
/// It does not stop trying at the budget: a full disk is usually someone
/// about to delete something, and a store that gave up could not heal when
/// they did. The budget bounds the *silence* instead. A streak outliving it
/// escalates once - waking anyone awaiting this flush with a failure, then
/// asking `policy.on_giveup` what writers should be told - and the loop
/// carries on regardless, so a flush that lands afterwards clears the failure
/// and the store is whole again with nothing restarted.
///
/// Only [`AfterGivingUp::Poison`] ends the thread, by panicking: `Drop`'s own
/// guard poisons on the way down, which is the same mechanism a panic inside
/// `op` itself already relies on.
///
/// A `Schedule` or a `Now` arriving mid-streak just retries sooner, and which
/// of the two it was is discarded. The two ways out of a streak that never
/// lands are a `Stop` and a disconnect - without them, `shutdown` would join a
/// thread still politely waiting for a disk that is never coming back.
fn run_with_retry(
    op: &mut dyn FnMut() -> StorageResult<()>,
    policy: &FlushPolicy,
    rx: &mpsc::Receiver<Trigger>,
    last: bool,
) -> Next {
    if last {
        let landed = op();
        policy.commits.finished(landed.is_ok());
        if landed.is_ok() {
            policy.health.landed();
        }
        return Next::Stop;
    }

    let retry = &policy.retry;
    let mut streak = Streak::Fresh;

    loop {
        let reason = match op() {
            Ok(()) => {
                policy.health.landed();
                policy.commits.finished(true);
                return Next::Wake;
            }
            Err(why) => Arc::new(why),
        };

        streak = match streak {
            Streak::Fresh => Streak::Failing {
                since: Instant::now(),
            },
            carried => carried,
        };

        if let Streak::Failing { since } = streak {
            let elapsed = since.elapsed();

            if elapsed >= retry.budget {
                give_up(&reason, elapsed, policy);
                streak = Streak::GaveUp;
            } else {
                warn!(
                    target: "amethystate",
                    kind = ?reason.current_context(),
                    elapsed_ms = elapsed.as_millis() as u64,
                    budget_ms = retry.budget.as_millis() as u64,
                    "background flush failed, retrying",
                );
            }
        }

        match rx.recv_timeout(retry.interval) {
            Err(RecvTimeoutError::Disconnected) => {
                debug!("debouncer thread leaving a failing flush: the store is gone");
                return Next::Wake;
            }
            Ok(Trigger::Stop) => {
                debug!("debouncer thread leaving a failing flush: asked to stop");
                return Next::Stop;
            }
            Ok(Trigger::Schedule) | Ok(Trigger::Now) | Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_poison_on_op_panic() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let count_inner = call_count.clone();

        let d = Debouncer::new(Duration::from_millis(50), move || {
            let n = count_inner.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                panic!("simulated failure");
            }
        });

        assert!(!d.is_poisoned());

        d.schedule();
        d.wait_dead();

        assert!(d.is_poisoned());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_pending_op_runs_when_dropped_mid_interval() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let count_inner = call_count.clone();

        let d = Debouncer::new(Duration::from_secs(30), move || {
            count_inner.fetch_add(1, Ordering::SeqCst);
        });

        d.schedule();
        drop(d);

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_idle_drop_runs_nothing() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let count_inner = call_count.clone();

        let d = Debouncer::new(Duration::from_millis(10), move || {
            count_inner.fetch_add(1, Ordering::SeqCst);
        });

        drop(d);

        assert_eq!(call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_schedule_panics_when_poisoned() {
        let d = Debouncer::new(Duration::from_millis(50), move || {
            panic!("simulated failure");
        });

        d.schedule();
        d.wait_dead();

        assert!(d.is_poisoned());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| d.schedule()));
        assert!(result.is_err());
    }
}
