use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
use tracing::debug;

/// Why the thread was woken: `Schedule` opens a window if none is open, `Now`
/// cuts the window short for a caller that is waiting on the commit, and `Stop`
/// ends the thread.
///
/// `Stop` travels the same channel as the rest, so anything already queued
/// runs before it does - which is what lets a caller send a flush, then stop,
/// and learn from the join that the flush finished.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Trigger {
    Schedule,
    Now,
    Stop,
}

/// What the thread does once the work returns.
///
/// A retrying flush reads the channel between attempts, so a `Stop` can arrive
/// where [`next_state`] cannot see it. Handing it back is what keeps that stop
/// from being swallowed by a flush that is failing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Next {
    Wake,
    Stop,
}

/// Where the thread is.
///
/// `Settling` carries when its window closes. The window is opened by the
/// first write after a flush and is not moved by the ones after it, so writes
/// that never pause are still flushed once a window rather than never.
///
/// `Flushing` carries what to do once the work returns, because the two ways
/// into it differ only in that: a window that ran out goes back to waiting,
/// and one cut short by a `Stop` does not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum State {
    Idle,
    Settling { until: Instant },
    Flushing { then_stop: bool },
    Stopped,
}

/// One transition.
///
/// Every wait the thread does happens here, so `Stop` is read at each of them
/// and a stop cannot be missed by a thread that happens to be asleep.
///
/// `run` is told whether this is the last pass. A flush reached through a stop
/// has already consumed the `Stop` that would have ended it, so a retrying one
/// left to its own devices would try for as long as the disk stays broken,
/// with nobody able to call it off and a caller waiting on the join.
pub(super) fn next_state(
    state: State,
    rx: &mpsc::Receiver<Trigger>,
    interval: Duration,
    run: &mut dyn FnMut(&mpsc::Receiver<Trigger>, bool) -> Next,
) -> State {
    match state {
        State::Idle => match rx.recv() {
            Ok(Trigger::Schedule) => State::Settling {
                until: Instant::now() + interval,
            },
            Ok(Trigger::Now) => {
                debug!("debouncer trigger: asked for immediately");
                State::Flushing { then_stop: false }
            }
            Ok(Trigger::Stop) | Err(_) => State::Stopped,
        },

        State::Settling { until } => {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                debug!("debouncer trigger: window closed");
                return State::Flushing { then_stop: false };
            }

            match rx.recv_timeout(left) {
                Ok(Trigger::Schedule) => State::Settling { until },
                Ok(Trigger::Now) | Err(RecvTimeoutError::Timeout) => {
                    debug!("debouncer trigger: window closed");
                    State::Flushing { then_stop: false }
                }
                Ok(Trigger::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    debug!("debouncer trigger: a window cut short by a stop");
                    State::Flushing { then_stop: true }
                }
            }
        }

        State::Flushing { then_stop } => match run(rx, then_stop) {
            Next::Stop => State::Stopped,
            Next::Wake if then_stop => State::Stopped,
            Next::Wake => State::Idle,
        },

        State::Stopped => State::Stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: Duration = Duration::from_millis(10);

    struct Work {
        runs: usize,
        answer: Next,
        told_it_was_last: Option<bool>,
    }

    impl Work {
        fn answering(answer: Next) -> Self {
            Self {
                runs: 0,
                answer,
                told_it_was_last: None,
            }
        }
    }

    fn settling() -> State {
        State::Settling {
            until: Instant::now() + QUIET,
        }
    }

    fn step(state: State, sent: &[Trigger], work: &mut Work) -> State {
        let (tx, rx) = mpsc::channel();
        for trigger in sent {
            tx.send(*trigger).unwrap();
        }
        next_state(state, &rx, QUIET, &mut |_, last| {
            work.runs += 1;
            work.told_it_was_last = Some(last);
            work.answer
        })
    }

    fn stepping_over(state: State, sent: &[Trigger]) -> State {
        step(state, sent, &mut Work::answering(Next::Wake))
    }

    fn hung_up(state: State, work: &mut Work) -> State {
        let (tx, rx) = mpsc::channel::<Trigger>();
        drop(tx);
        next_state(state, &rx, QUIET, &mut |_, last| {
            work.runs += 1;
            work.told_it_was_last = Some(last);
            work.answer
        })
    }

    #[test]
    fn a_schedule_opens_a_window() {
        assert!(matches!(
            stepping_over(State::Idle, &[Trigger::Schedule]),
            State::Settling { .. }
        ));
    }

    #[test]
    fn a_further_schedule_leaves_the_window_where_the_first_one_put_it() {
        let open = settling();
        assert_eq!(stepping_over(open, &[Trigger::Schedule]), open);
    }

    #[test]
    fn a_window_that_has_closed_writes_even_with_a_schedule_waiting() {
        let closed = State::Settling {
            until: Instant::now() - QUIET,
        };
        assert_eq!(
            stepping_over(closed, &[Trigger::Schedule]),
            State::Flushing { then_stop: false }
        );
    }

    #[test]
    fn asking_for_it_now_skips_the_window() {
        assert_eq!(
            stepping_over(State::Idle, &[Trigger::Now]),
            State::Flushing { then_stop: false }
        );
        assert_eq!(
            stepping_over(settling(), &[Trigger::Now]),
            State::Flushing { then_stop: false }
        );
    }

    #[test]
    fn a_window_that_runs_out_writes() {
        assert_eq!(
            stepping_over(settling(), &[]),
            State::Flushing { then_stop: false }
        );
    }

    #[test]
    fn stopping_an_idle_thread_ends_it() {
        assert_eq!(stepping_over(State::Idle, &[Trigger::Stop]), State::Stopped);
    }

    #[test]
    fn stopping_an_open_window_writes_what_it_was_holding() {
        assert_eq!(
            stepping_over(settling(), &[Trigger::Stop]),
            State::Flushing { then_stop: true }
        );
    }

    #[test]
    fn a_write_that_lands_goes_back_to_waiting() {
        let mut work = Work::answering(Next::Wake);
        assert_eq!(
            step(State::Flushing { then_stop: false }, &[], &mut work),
            State::Idle
        );
        assert_eq!(work.runs, 1);
    }

    #[test]
    fn a_write_asked_to_stop_ends_the_thread_once_it_has_run() {
        let mut work = Work::answering(Next::Wake);
        assert_eq!(
            step(State::Flushing { then_stop: true }, &[], &mut work),
            State::Stopped
        );
        assert_eq!(work.runs, 1);
    }

    #[test]
    fn a_write_reached_through_a_stop_is_told_it_is_the_last() {
        let mut work = Work::answering(Next::Wake);
        step(State::Flushing { then_stop: true }, &[], &mut work);
        assert_eq!(work.told_it_was_last, Some(true));

        let mut ordinary = Work::answering(Next::Wake);
        step(State::Flushing { then_stop: false }, &[], &mut ordinary);
        assert_eq!(ordinary.told_it_was_last, Some(false));
    }

    #[test]
    fn a_failing_write_that_was_stopped_ends_the_thread() {
        let mut work = Work::answering(Next::Stop);
        assert_eq!(
            step(State::Flushing { then_stop: false }, &[], &mut work),
            State::Stopped
        );
    }

    #[test]
    fn a_hung_up_channel_ends_an_idle_thread() {
        let mut work = Work::answering(Next::Wake);
        assert_eq!(hung_up(State::Idle, &mut work), State::Stopped);
        assert_eq!(work.runs, 0);
    }

    #[test]
    fn a_hung_up_channel_still_writes_what_an_open_window_was_holding() {
        let mut work = Work::answering(Next::Wake);
        assert_eq!(
            hung_up(settling(), &mut work),
            State::Flushing { then_stop: true }
        );
    }

    #[test]
    fn a_stopped_thread_stays_stopped_and_runs_nothing() {
        let mut work = Work::answering(Next::Wake);
        assert_eq!(
            step(
                State::Stopped,
                &[Trigger::Schedule, Trigger::Now],
                &mut work
            ),
            State::Stopped
        );
        assert_eq!(work.runs, 0);
    }

    #[test]
    fn a_stop_behind_a_queued_schedule_is_still_read() {
        let (tx, rx) = mpsc::channel();
        tx.send(Trigger::Schedule).unwrap();
        tx.send(Trigger::Stop).unwrap();

        let mut run = |_: &mpsc::Receiver<Trigger>, _: bool| Next::Wake;

        let open = next_state(State::Idle, &rx, QUIET, &mut run);
        assert!(
            matches!(open, State::Settling { .. }),
            "the schedule was read first"
        );

        assert_eq!(
            next_state(open, &rx, QUIET, &mut run),
            State::Flushing { then_stop: true },
            "the stop waited behind the schedule and must be read at the next \
             wait, not left in the channel for the window to time out over"
        );
    }
}
