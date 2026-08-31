use arc_swap::ArcSwap;
use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

type SignalCallback<T> = Arc<dyn Fn(&T, Option<Uuid>) + Send + Sync + 'static>;
type SubscriberEntry<T> = (u64, SignalCallback<T>, SubscriptionMeta);
type SignalSubscribers<T> = Arc<Mutex<Vec<SubscriberEntry<T>>>>;

#[derive(Clone, Copy)]
pub struct SubscriptionMeta {
    pub id: u64,
    pub location: &'static Location<'static>,
    pub name: Option<&'static str>,
}

pub struct Signal<T> {
    pub value: Arc<ArcSwap<T>>,
    pub subscribers: SignalSubscribers<T>,
    pub next_id: Arc<AtomicU64>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            next_id: self.next_id.clone(),
            subscribers: self.subscribers.clone(),
        }
    }
}

/// Dropping this ends the subscription, so it has to be held for as long as the
/// callback should keep firing.
///
/// This is the right to end one subscription, and it is deliberately not
/// `Clone`: a copy of that right would be a second way to end the same
/// subscription. Hand it to one place; `ReactiveScope` is where several are
/// held together.
#[must_use = "dropping a subscription unsubscribes; bind it to keep it alive"]
pub struct SignalSubscription {
    pub id: u64,
    pub location: &'static Location<'static>,
    pub name: Option<&'static str>,
    pub set_name: Arc<dyn Fn(&'static str) + Send + Sync + 'static>,
    pub cleanup: Arc<dyn Fn(u64) + Send + Sync + 'static>,
}

impl SignalSubscription {
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        (self.set_name)(name);
        self
    }

    pub fn watch(self, scope: &mut ReactiveScope) {
        scope.watch(self);
    }
}

impl Drop for SignalSubscription {
    fn drop(&mut self) {
        (self.cleanup)(self.id);
    }
}

/// Subscriptions held together, ending when the scope does.
///
/// Not `Clone`, for the reason [`SignalSubscription`] is not: a copy of the
/// scope would be a second chance to end every subscription in it, and the
/// first copy dropped would take them all.
#[derive(Default)]
pub struct ReactiveScope {
    subs: Vec<SignalSubscription>,
}

impl ReactiveScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn watch(&mut self, sub: SignalSubscription) {
        self.subs.push(sub);
    }

    pub fn watch_scope(&mut self, mut other: Self) {
        self.subs.append(&mut other.subs);
    }

    pub fn clear(&mut self) {
        self.subs.clear();
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }
}

impl<T: 'static> Signal<T> {
    pub fn new(initial: T) -> Self {
        Self {
            value: Arc::new(ArcSwap::from_pointee(initial)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Writes a value that originates here.
    pub fn set(&self, value: T) {
        self.store_and_emit(value, None);
    }

    /// Writes a value that originates elsewhere, carrying its provenance.
    ///
    /// For propagation layers - a store subscription applying a committed
    /// change, an interceptor rewriting one - so subscribers can tell whose
    /// write they are seeing. Application code wants [`Signal::set`].
    pub fn set_with_source(&self, value: T, source: Uuid) {
        self.store_and_emit(value, Some(source));
    }

    /// Re-emits a change with whatever provenance it already carried.
    ///
    /// Only for layers forwarding someone else's change, where "no provenance"
    /// is a real possibility - an edit made to the file outside this process
    /// arrives with none. Everyone else picks [`Signal::set`] or
    /// [`Signal::set_with_source`] explicitly.
    #[doc(hidden)]
    pub fn set_forwarded(&self, value: T, source: Option<Uuid>) {
        self.store_and_emit(value, source);
    }

    fn store_and_emit(&self, value: T, source: Option<Uuid>) {
        let value = Arc::new(value);
        self.value.store(Arc::clone(&value));
        self.emit(value, source);
    }

    fn emit(&self, value: Arc<T>, source: Option<Uuid>) {
        let callbacks: Vec<_> = {
            let subs = self.subscribers.lock().unwrap();
            subs.iter()
                .map(|(_, cb, meta)| (cb.clone(), *meta))
                .collect()
        };
        for (cb, meta) in callbacks {
            tracing::trace!(
                target: "amethystate",
                subscription_id = meta.id,
                name = meta.name,
                location = format!("{}:{}", meta.location.file(), meta.location.line()),
                "signal emit → subscription fire",
            );
            cb(&value, source);
        }
    }

    #[track_caller]
    pub fn subscribe<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&T) + Send + Sync + 'static,
    {
        self.subscribe_with_source(move |val, _src| callback(val))
    }

    #[track_caller]
    pub fn subscribe_with_source<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&T, Option<Uuid>) + Send + Sync + 'static,
    {
        let location = Location::caller();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let meta = SubscriptionMeta {
            id,
            location,
            name: None,
        };
        {
            let mut subs = self.subscribers.lock().unwrap();
            subs.push((id, Arc::new(callback), meta));
        }

        let subscribers_for_name = self.subscribers.clone();
        let set_name = Arc::new(move |name: &'static str| {
            if let Ok(mut subs) = subscribers_for_name.lock()
                && let Some(entry) = subs.iter_mut().find(|(sid, _, _)| *sid == id)
            {
                entry.2.name = Some(name);
            }
        });

        let subscribers_for_cleanup = self.subscribers.clone();
        SignalSubscription {
            id,
            location,
            name: None,
            set_name,
            cleanup: Arc::new(move |id| {
                if let Ok(mut subs) = subscribers_for_cleanup.lock() {
                    subs.retain(|(sid, _, _)| *sid != id);
                }
            }),
        }
    }
}

impl<T: Clone + 'static> Signal<T> {
    pub fn get(&self) -> T {
        self.value.load().as_ref().clone()
    }
}

impl<T> Signal<T> {
    /// Whether these are two handles on the same signal, rather than two
    /// signals holding equal values.
    ///
    /// An associated function like [`Arc::ptr_eq`], and named after it, because
    /// `a == b` would read as a question about the values. It is the whole of
    /// what a caller needs to compare handles, so none of them has to know how
    /// a signal is put together.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.value, &b.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn signal_subscription_cleanup_on_drop() {
        let signal = Signal::new("a".to_string());
        let counter = Arc::new(Mutex::new(0usize));
        {
            let cap = counter.clone();
            let _sub = signal.subscribe(move |_: &String| {
                *cap.lock().unwrap() += 1;
            });
            signal.set("b".to_string());
            assert_eq!(*counter.lock().unwrap(), 1);
        }
        signal.set("c".to_string());
        assert_eq!(*counter.lock().unwrap(), 1);
    }

    #[test]
    fn named_updates_meta_in_subscribers() {
        let signal = Signal::new(0i32);
        let _sub = signal.subscribe(|_| {}).named("MyWatcher");

        let subs = signal.subscribers.lock().unwrap();
        assert_eq!(subs[0].2.name, Some("MyWatcher"));
    }

    /// The location is the caller's, not this function's.
    ///
    /// `Location::caller().file()` is never empty, so asserting that it is not
    /// says nothing: without `#[track_caller]` the location would simply point
    /// at `signal.rs` instead, which is where this test lives too. The line is
    /// what tells the two apart.
    #[test]
    fn subscription_location_captured() {
        let signal = Signal::new(0i32);
        let here = line!();
        let sub = signal.subscribe(|_| {});

        assert_eq!(sub.location.file(), file!());
        assert_eq!(
            sub.location.line(),
            here + 1,
            "the subscription recorded where it was made rather than where the \
             call landed - `#[track_caller]` is missing from `subscribe`"
        );
    }

    #[test]
    fn concurrent_writes_keep_value_and_source_together() {
        const WRITERS: usize = 8;
        const WRITES: usize = 500;

        let signal = Signal::new((Uuid::nil(), 0usize));
        let mismatches = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let seen = mismatches.clone();
        let _sub = signal.subscribe_with_source(move |(stamp, _): &(Uuid, usize), source| {
            if Some(*stamp) != source {
                seen.fetch_add(1, Ordering::Relaxed);
            }
        });

        std::thread::scope(|scope| {
            for _ in 0..WRITERS {
                let signal = signal.clone();
                scope.spawn(move || {
                    let me = Uuid::new_v4();
                    for n in 0..WRITES {
                        signal.set_with_source((me, n), me);
                    }
                });
            }
        });

        assert_eq!(
            mismatches.load(Ordering::Relaxed),
            0,
            "value and source must describe the same write"
        );
    }
}
