use crate::store::util::DeadNotifier;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;
use tracing::debug;

/// Runs `op` on a fixed interval until dropped.
/// If `op` panics, the guard mutex is poisoned — detectable via `is_poisoned()`.
pub struct Ticker {
    tx: Option<mpsc::Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
    guard: Arc<Mutex<()>>,
    #[cfg(test)]
    dead: Arc<(Mutex<bool>, Condvar)>,
}

impl Ticker {
    pub fn new<F>(interval: Duration, mut op: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<()>();
        let guard = Arc::new(Mutex::new(()));
        let dead = Arc::new((Mutex::new(false), Condvar::new()));
        let guard_inner = guard.clone();
        let dead_inner = dead.clone();

        let handle = thread::spawn(move || {
            let _notify = DeadNotifier(dead_inner);
            let _hold = guard_inner.lock().unwrap();

            loop {
                match rx.recv_timeout(interval) {
                    Ok(()) => {
                        debug!("ticker thread: stop signal received");
                        return;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        debug!("ticker trigger: interval elapsed");
                        op();
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        debug!("ticker thread exiting (channel closed)");
                        return;
                    }
                }
            }
        });

        Self {
            tx: Some(tx),
            handle: Some(handle),
            guard,
            #[cfg(test)]
            dead,
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.guard.is_poisoned()
    }

    #[cfg(test)]
    pub fn wait_dead(&self) {
        let (lock, cvar) = &*self.dead;
        let _unused = cvar.wait_while(lock.lock().unwrap(), |dead| !*dead);
    }

    fn shutdown(&mut self) {
        if let Some(ref tx) = self.tx {
            let _ = tx.send(());
        }
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_ticks_regularly() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_inner = count.clone();

        let t = Ticker::new(Duration::from_millis(30), move || {
            count_inner.fetch_add(1, Ordering::SeqCst);
        });

        thread::sleep(Duration::from_millis(600));
        drop(t);

        let n = count.load(Ordering::SeqCst);
        assert!(n >= 3, "expected at least 3 ticks, got {n}");
    }

    #[test]
    fn test_poison_on_op_panic() {
        let t = Ticker::new(Duration::from_millis(30), move || {
            panic!("simulated failure");
        });

        t.wait_dead();
        assert!(t.is_poisoned());
    }

    #[test]
    fn test_no_ticks_after_drop() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_inner = count.clone();

        let t = Ticker::new(Duration::from_millis(30), move || {
            count_inner.fetch_add(1, Ordering::SeqCst);
        });

        drop(t);

        let after_drop = count.load(Ordering::SeqCst);
        thread::sleep(Duration::from_millis(150));
        let after_sleep = count.load(Ordering::SeqCst);

        assert_eq!(after_drop, after_sleep, "ticks continued after drop");
    }
}
