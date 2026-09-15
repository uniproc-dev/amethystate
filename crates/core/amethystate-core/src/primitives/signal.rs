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

/// Drops the entry a subscription was registered as.
///
/// The two things a [`SignalSubscription`] does to the list it came from, over
/// whichever list that is: a signal keeps one, a map keeps one for every-key
/// subscribers and one per key. The callback type differs and the bookkeeping
/// does not.
pub(crate) fn forget<C>(list: &mut Vec<(u64, C, SubscriptionMeta)>, id: u64) {
    list.retain(|(at, _, _)| *at != id);
}

/// The same, for the name a caller gave a subscription after taking it out.
pub(crate) fn label<C>(list: &mut [(u64, C, SubscriptionMeta)], id: u64, name: &'static str) {
    if let Some(entry) = list.iter_mut().find(|(at, _, _)| *at == id) {
        entry.2.name = Some(name);
    }
}

/// A subscriber list, whether or not a panic went through while it was held.
///
/// A panic between two statements of a `Vec`'s own leaves a `Vec` that is still
/// one, so a poisoned lock here carries nothing a reader could act on. Taking
/// it anyway is what keeps one subscriber's panic from reaching every other:
/// panicking in turn spreads it, and reading the list as empty answers "the
/// lock is broken" with "nobody is listening" and then delivers to nobody for
/// the life of the process.
pub(crate) fn held<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A value and where it comes in the order writes were made.
pub struct Stamped<T> {
    pub at: u64,
    pub value: T,
}

/// A write that landed and has not been announced.
type Announcing<T> = Mutex<Option<(Arc<Stamped<T>>, Option<Uuid>)>>;

pub struct Signal<T> {
    value: Arc<ArcSwap<Stamped<T>>>,
    pub subscribers: SignalSubscribers<T>,
    pub next_id: Arc<AtomicU64>,

    /// The order writes are settled in, minted here because this is where a
    /// value is put in place.
    ticket: Arc<AtomicU64>,

    /// Held by whoever is announcing, so subscribers are called in the order
    /// the values landed rather than in the order the writers woke up.
    announcing: Arc<Mutex<()>>,

    /// What landed while somebody else was announcing, for them to pick up
    /// rather than for its own writer to wait on.
    waiting: Arc<Announcing<T>>,
    announced: Arc<AtomicU64>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            next_id: self.next_id.clone(),
            subscribers: self.subscribers.clone(),
            ticket: self.ticket.clone(),
            announcing: self.announcing.clone(),
            waiting: self.waiting.clone(),
            announced: self.announced.clone(),
        }
    }
}

/// Dropping this ends the subscription, so it has to be held for as long as the
/// callback should keep firing. [`ReactiveScope`] is where several are held
/// together.
#[must_use = "dropping a subscription unsubscribes; bind it to keep it alive"]
pub struct SignalSubscription {
    id: u64,
    location: &'static Location<'static>,
    name: Option<&'static str>,
    set_name: Arc<dyn Fn(&'static str) + Send + Sync + 'static>,
    cleanup: Arc<dyn Fn(u64) + Send + Sync + 'static>,
}

impl SignalSubscription {
    pub(crate) fn new(
        id: u64,
        location: &'static Location<'static>,
        set_name: Arc<dyn Fn(&'static str) + Send + Sync + 'static>,
        cleanup: Arc<dyn Fn(u64) + Send + Sync + 'static>,
    ) -> Self {
        Self {
            id,
            location,
            name: None,
            set_name,
            cleanup,
        }
    }

    /// Where the subscription was taken out, for a log that has to say which
    /// one it means.
    pub fn location(&self) -> &'static Location<'static> {
        self.location
    }

    pub fn name(&self) -> Option<&'static str> {
        self.name
    }

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
            value: Arc::new(ArcSwap::from_pointee(Stamped {
                at: 0,
                value: initial,
            })),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(0)),
            ticket: Arc::new(AtomicU64::new(0)),
            announcing: Arc::new(Mutex::new(())),
            waiting: Arc::new(Mutex::new(None)),
            announced: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Writes a value that originates here.
    pub fn set(&self, value: T) {
        self.set_forwarded(value, None);
    }

    /// Writes a value that originates elsewhere, carrying its provenance.
    ///
    /// For propagation layers - a store subscription applying a committed
    /// change, an interceptor rewriting one - so subscribers can tell whose
    /// write they are seeing. Application code wants [`Signal::set`].
    pub fn set_with_source(&self, value: T, source: Uuid) {
        self.set_forwarded(value, Some(source));
    }

    /// Re-emits a change with whatever provenance it already carried.
    ///
    /// Only for layers forwarding someone else's change, where "no provenance"
    /// is a real possibility - an edit made to the file outside this process
    /// arrives with none. Everyone else picks [`Signal::set`] or
    /// [`Signal::set_with_source`] explicitly.
    #[doc(hidden)]
    pub fn set_forwarded(&self, value: T, source: Option<Uuid>) {
        let at = self.ticket.fetch_add(1, Ordering::AcqRel) + 1;
        self.store_and_emit(Stamped { at, value }, source);
    }

    /// The same, in the order somebody else settled.
    ///
    /// For a value that comes from a store: the store decided which of two
    /// racing writes is the later one, and a signal that minted its own number
    /// on arrival would decide it again, differently, whenever the callbacks
    /// happened to run out of order.
    #[doc(hidden)]
    pub fn set_settled(&self, value: T, source: Option<Uuid>, at: u64) {
        // One order, not two. A signal fed by a store is still writable
        // directly - `Signal::set` is public, and a field hands its signal out -
        // so a number minted here has to leave the local one above it, or the
        // next write of one's own is older than what is already held and never
        // lands.
        self.ticket.fetch_max(at, Ordering::AcqRel);
        self.store_and_emit(Stamped { at, value }, source);
    }

    /// Puts the value in place and tells whoever is subscribed, in the order
    /// the values landed.
    ///
    /// A write that lost the race neither lands nor is announced: what a
    /// signal holds is the last write settled, and the last call a subscriber
    /// gets is that same value. A write arriving while somebody is announcing
    /// leaves its place in the queue rather than waiting for the lock, so a
    /// subscriber writing from inside its own callback is no different from
    /// any other writer.
    fn store_and_emit(&self, value: Stamped<T>, source: Option<Uuid>) {
        let at = value.at;
        let landed = Arc::new(value);

        if !self.land(&landed) {
            return;
        }

        {
            let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
            let queued = waiting.as_ref().map(|(held, _)| held.at).unwrap_or(0);

            if queued < at {
                *waiting = Some((landed, source));
            }
        }

        self.announce();
    }

    /// Puts `next` in place unless something later is already there.
    fn land(&self, next: &Arc<Stamped<T>>) -> bool {
        loop {
            let held = self.value.load();
            if held.at > next.at {
                return false;
            }

            let previous = self.value.compare_and_swap(&held, Arc::clone(next));
            if Arc::ptr_eq(&previous, &held) {
                return true;
            }
        }
    }

    /// Announces everything that has landed and not been announced, until
    /// nothing is left or somebody else is already doing it.
    fn announce(&self) {
        loop {
            {
                let Ok(_in_order) = self.announcing.try_lock() else {
                    return;
                };

                while let Some((landed, source)) = self.next_to_announce() {
                    self.announced.store(landed.at, Ordering::Release);
                    self.emit(landed, source);
                }
            }

            let queued = self
                .waiting
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            if !queued {
                return;
            }
        }
    }

    fn next_to_announce(&self) -> Option<(Arc<Stamped<T>>, Option<Uuid>)> {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());

        match waiting.take() {
            Some((held, _)) if held.at <= self.announced.load(Ordering::Acquire) => None,
            held => held,
        }
    }

    fn emit(&self, value: Arc<Stamped<T>>, source: Option<Uuid>) {
        let callbacks: Vec<_> = held(&self.subscribers)
            .iter()
            .map(|(_, cb, meta)| (cb.clone(), *meta))
            .collect();
        for (cb, meta) in callbacks {
            tracing::trace!(
                target: "amethystate",
                subscription_id = meta.id,
                name = meta.name,
                location = format!("{}:{}", meta.location.file(), meta.location.line()),
                "signal emit → subscription fire",
            );
            cb(&value.value, source);
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
        held(&self.subscribers).push((id, Arc::new(callback), meta));

        let subscribers_for_name = self.subscribers.clone();
        let set_name = Arc::new(move |name: &'static str| {
            label(&mut held(&subscribers_for_name), id, name);
        });

        let subscribers_for_cleanup = self.subscribers.clone();
        SignalSubscription::new(
            id,
            location,
            set_name,
            Arc::new(move |id| {
                forget(&mut held(&subscribers_for_cleanup), id);
            }),
        )
    }
}

impl<T: Clone + 'static> Signal<T> {
    pub fn get(&self) -> T {
        self.value.load().value.clone()
    }
}

impl<T: 'static> Signal<T> {
    /// What the signal holds, borrowed rather than cloned, with the place it
    /// takes in the order writes were settled.
    pub fn held(&self) -> arc_swap::Guard<Arc<Stamped<T>>> {
        self.value.load()
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
    fn a_value_settled_earlier_does_not_land_on_a_later_one() {
        let signal = Signal::new(0u64);
        let seen = Arc::new(Mutex::new(Vec::new()));

        let heard = seen.clone();
        let _sub = signal.subscribe(move |v: &u64| heard.lock().unwrap().push(*v));

        signal.set_settled(2, None, 2);
        signal.set_settled(1, None, 1);

        assert_eq!(
            signal.get(),
            2,
            "the store settled 1 before 2, and the callbacks arrived the other way round"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![2],
            "the older value was announced after the newer one, so a subscriber ends on it"
        );

        signal.set(3);

        assert_eq!(
            signal.get(),
            3,
            "a write of this signal's own after one settled elsewhere is the later of the two, \
             and mints above it rather than starting again from one"
        );
    }

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
