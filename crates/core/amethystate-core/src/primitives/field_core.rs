use crate::change::Change;
use crate::path::StorePath;
use crate::primitives::intercept::{InterceptDisposer, InterceptGuard};
use crate::primitives::signal::{Signal, SignalSubscription};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub trait FieldValue: DeserializeOwned + Serialize + Clone + Send + Sync + 'static {}
impl<T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static> FieldValue for T {}

pub struct FieldCore<T> {
    pub signal: Signal<T>,
    pub interceptors: Arc<
        Mutex<
            Vec<(
                u64,
                Arc<dyn Fn(Change<T>) -> Option<Change<T>> + Send + Sync + 'static>,
            )>,
        >,
    >,
    pub next_interceptor_id: Arc<AtomicUsize>,
    pub intercept_depth: Arc<AtomicUsize>,
}

impl<T> Clone for FieldCore<T> {
    fn clone(&self) -> Self {
        Self {
            signal: self.signal.clone(),
            interceptors: self.interceptors.clone(),
            next_interceptor_id: self.next_interceptor_id.clone(),
            intercept_depth: self.intercept_depth.clone(),
        }
    }
}

impl<T: Clone + 'static> FieldCore<T> {
    pub fn new_with_signal(initial: Signal<T>) -> Self {
        Self {
            signal: initial,
            interceptors: Arc::new(Mutex::new(Vec::new())),
            next_interceptor_id: Arc::new(AtomicUsize::new(0)),
            intercept_depth: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn new(initial: T) -> Self {
        Self {
            signal: Signal::new(initial),
            interceptors: Arc::new(Mutex::new(Vec::new())),
            next_interceptor_id: Arc::new(AtomicUsize::new(0)),
            intercept_depth: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get(&self) -> T {
        self.signal.get()
    }

    #[track_caller]
    pub fn subscribe<F>(&self, callback: F) -> SignalSubscription
    where
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.signal.subscribe(callback)
    }

    #[track_caller]
    pub fn subscribe_with_source<F>(&self, callback: F) -> SignalSubscription
    where
        F: for<'a> Fn(&'a T, Option<Uuid>) + Send + Sync + 'static,
    {
        self.signal.subscribe_with_source(callback)
    }

    pub fn intercept<F>(&self, path: StorePath, callback: F) -> InterceptDisposer
    where
        F: Fn(Change<T>) -> Option<Change<T>> + Send + Sync + 'static,
    {
        let id = self.next_interceptor_id.fetch_add(1, Ordering::Relaxed);
        self.interceptors
            .lock()
            .unwrap()
            .push((id as u64, Arc::new(callback)));

        let interceptors = self.interceptors.clone();
        InterceptDisposer {
            id: id as u64,
            path: path.clone(),
            cleanup: Arc::new(move |id| {
                if let Ok(mut lock) = interceptors.lock() {
                    lock.retain(|(i, _)| *i != id);
                }
            }),
        }
    }

    pub fn run_interceptors(
        &self,
        path: StorePath,
        value: T,
        source: Option<Uuid>,
    ) -> Result<Change<T>, String> {
        let mut change = Change {
            source,
            old_value: self.get(),
            new_value: value,
        };

        let Some(_guard) = InterceptGuard::enter(&self.intercept_depth, path) else {
            return Err("interceptors nested too deep".to_string());
        };

        let interceptors = { self.interceptors.lock().unwrap().clone() };
        for (_, interceptor) in interceptors {
            if let Some(new_change) = interceptor(change.clone()) {
                change = new_change;
            } else {
                return Err("refused by an interceptor on the field".to_string());
            }
        }

        Ok(change)
    }
}
