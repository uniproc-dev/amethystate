use crate::reactive::map::KeyOf;
use amethystate_core::SignalSubscription;
use futures_core::Stream;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use uuid::Uuid;

/// Crosses the thread boundary: the writer flags it, the polling thread waits
/// on it. Carries no value - the queue does that.
#[derive(Default)]
struct Wake {
    waker: Mutex<Option<Waker>>,
}

impl Wake {
    fn signal(&self) {
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }

    /// Registers `cx`'s waker. Callers must re-check their queue afterwards: a
    /// signal landing between their check and this one would otherwise be
    /// missed, with nothing left to wake them.
    fn park(&self, cx: &Context<'_>) {
        *self.waker.lock().unwrap() = Some(cx.waker().clone());
    }
}

/// Something a [`Watch`] can be built over.
pub trait Watchable {
    /// What a subscriber is handed.
    type Item: Clone + Send + 'static;

    /// Identity of the handle, so `external` can tell its own writes apart.
    fn watch_id(&self) -> Uuid;

    /// Whether [`Watch::external`] may drop this item when it came from this
    /// handle.
    ///
    /// Maps say no for anything but `Update`: rewriting a value is the business
    /// of whoever wrote it, but a key appearing or disappearing changes what the
    /// map holds and goes to everyone, including the handle that caused it.
    fn filterable(_item: &Self::Item) -> bool {
        true
    }

    /// The location an implementation records is the one a trace shows, so it
    /// is the caller's.
    #[track_caller]
    fn watch_raw<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&Self::Item, Option<Uuid>) + Send + Sync + 'static;
}

/// A subscription being configured.
///
/// Built by `subscription_with` on the primitive, finished by [`Watch::register`],
/// [`Watch::register_with_source`] or [`Watch::stream`]. Without a terminal call nothing is
/// subscribed.
#[must_use = "a Watch subscribes to nothing until register() is called"]
pub struct Watch<W> {
    source: W,
    external: bool,
}

impl<W: Watchable> Watch<W> {
    pub(crate) fn new(source: W) -> Self {
        Self {
            source,
            external: false,
        }
    }

    /// Installs the callback with everything configured so far, and yields
    /// the guard that keeps it alive.
    ///
    /// Dropping the returned guard unsubscribes.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # use amethystate::store::field_with_path;
    /// # use std::sync::Arc;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let port = field_with_path::<u16>(
    ///     &store, ["net", "port"], 8080, amethystate::uuid::Uuid::new_v4(),
    /// ).unwrap();
    /// # use std::sync::Mutex;
    /// let seen = Arc::new(Mutex::new(Vec::new()));
    /// let sink = Arc::clone(&seen);
    ///
    /// let sub = port.subscription_with().register(move |v| {
    ///     sink.lock().unwrap().push(*v);
    /// });
    ///
    /// port.set(9090).unwrap();
    /// port.set(9091).unwrap();
    /// assert_eq!(*seen.lock().unwrap(), [9090, 9091]);
    ///
    /// drop(sub);
    /// port.set(9092).unwrap();
    /// assert_eq!(seen.lock().unwrap().len(), 2, "the guard is what kept it alive");
    /// ```
    #[track_caller]
    pub fn register<F>(self, callback: F) -> SignalSubscription
    where
        F: Fn(&W::Item) + Send + Sync + 'static,
    {
        self.register_with_source(move |item, _| callback(item))
    }

    /// Installs a callback that also receives who made the change, so it
    /// can tell its own writes from anyone else's.
    ///
    /// [`Watch::external`] drops those changes before the callback runs, which
    /// is usually what you want; take the id yourself when a write of your own
    /// calls for its own handling.
    ///
    /// Dropping the returned guard unsubscribes.
    ///
    /// The example below uses [`Field::fork`](crate::Field::fork) rather than
    /// [`Clone`] to stand in for a second component: a clone shares the
    /// instance id, so its writes would arrive indistinguishable from the
    /// original's.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # use amethystate::store::field_with_path;
    /// # use std::sync::Arc;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let port = field_with_path::<u16>(
    ///     &store, ["net", "port"], 8080, amethystate::uuid::Uuid::new_v4(),
    /// ).unwrap();
    /// # use std::sync::Mutex;
    /// let port_fork = port.fork();
    ///
    /// let seen = Arc::new(Mutex::new(Vec::new()));
    /// let sink = Arc::clone(&seen);
    ///
    /// let _sub = port.subscription_with().register_with_source(move |v, src| {
    ///     sink.lock().unwrap().push((*v, src));
    /// });
    ///
    /// port.set(9090).unwrap();
    /// port_fork.set(9091).unwrap();
    ///
    /// let seen = seen.lock().unwrap();
    /// assert_eq!(seen.len(), 2, "both arrived");
    /// assert_ne!(seen[0].1, seen[1].1, "each carries who wrote it");
    /// ```
    #[track_caller]
    pub fn register_with_source<F>(self, callback: F) -> SignalSubscription
    where
        F: Fn(&W::Item, Option<Uuid>) + Send + Sync + 'static,
    {
        let mine = self.external.then(|| self.source.watch_id());

        self.source.watch_raw(move |item, source| {
            if mine.is_some() && source == mine && W::filterable(item) {
                return;
            }
            callback(item, source);
        })
    }
}

impl<W: Watchable> Watch<W> {
    /// A stream of changes instead of a callback.
    ///
    /// For consumers with a loop of their own: nothing has to be `Send + Sync`
    /// beyond the value itself, and no scope has to be drained. Every change is
    /// yielded - a stream is a sequence, so coalescing is left to whoever wants
    /// it.
    ///
    /// Dropping the stream ends the subscription.
    #[track_caller]
    pub fn stream(self) -> ChangeStream<W::Item> {
        let mine = self.external.then(|| self.source.watch_id());
        let queue: Arc<Mutex<VecDeque<W::Item>>> = Arc::new(Mutex::new(VecDeque::new()));
        let wake = Arc::new(Wake::default());

        let sink = Arc::clone(&queue);
        let signal = Arc::clone(&wake);

        let sub = self.source.watch_raw(move |item, source| {
            if mine.is_some() && source == mine && W::filterable(item) {
                return;
            }
            sink.lock().unwrap().push_back(item.clone());
            signal.signal();
        });

        ChangeStream {
            queue,
            wake,
            _sub: sub,
        }
    }
}

/// A [`Stream`] of changes, built by [`Watch::stream`].
#[must_use = "dropping the stream ends the subscription"]
pub struct ChangeStream<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
    wake: Arc<Wake>,
    _sub: SignalSubscription,
}

impl<T> Stream for ChangeStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        if let Some(item) = self.queue.lock().unwrap().pop_front() {
            return Poll::Ready(Some(item));
        }

        self.wake.park(cx);

        match self.queue.lock().unwrap().pop_front() {
            Some(item) => Poll::Ready(Some(item)),
            None => Poll::Pending,
        }
    }
}

impl<W> Watch<W> {
    /// Skips changes this handle made itself.
    pub fn external(mut self) -> Self {
        self.external = true;
        self
    }
}

impl<K, V> Watch<crate::ReactiveMap<K, V>>
where
    K: crate::ReactiveMapKey,
    V: crate::ReactiveMapValue,
{
    /// Narrows to one key instead of every change in the map.
    pub fn key(self, key: K) -> Watch<KeyOf<K, V>> {
        Watch {
            source: KeyOf::new(self.source, key),
            external: self.external,
        }
    }
}
