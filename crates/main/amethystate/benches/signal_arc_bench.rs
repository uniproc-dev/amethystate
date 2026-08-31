//! Measures what an `Arc` travelling a propagation chain would save.
//!
//! `Signal<T>` stores `Arc<T>` and hands subscribers `&T`. So a layer that
//! forwards a change onwards - a store subscription applying a committed write,
//! a callback pushing to a downstream signal - clones the `T` it was lent and
//! the next hop allocates a fresh `Arc` for it. Every hop pays a clone and a
//! malloc.
//!
//! Letting `Arc<T>` travel instead would pay neither. The question is whether
//! that is worth changing the subscriber signature, which is public API and
//! reaches every adapter.
//!
//! `ArcSignal` here is a stand-in for the proposed shape - it models the
//! handing-over, nothing else. The `Signal` on the other side is the real one.

use amethystate_core::Signal;
use arc_swap::ArcSwap;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::{Arc, Mutex};

// ---- proposed shape -------------------------------------------------------

/// A signal whose subscribers are lent the `Arc` rather than what is inside it.
struct ArcSignal<T> {
    value: Arc<ArcSwap<T>>,
    subscribers: Arc<Mutex<Vec<Arc<dyn Fn(&Arc<T>) + Send + Sync>>>>,
}

impl<T: Send + Sync + 'static> ArcSignal<T> {
    fn new(initial: T) -> Self {
        Self {
            value: Arc::new(ArcSwap::from_pointee(initial)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The whole point: nothing is cloned and nothing is allocated.
    fn set_arc(&self, value: Arc<T>) {
        self.value.store(Arc::clone(&value));
        let callbacks: Vec<_> = self.subscribers.lock().unwrap().iter().cloned().collect();
        for cb in callbacks {
            cb(&value);
        }
    }
}

/// A chain of `hops` signals, each forwarding to the next.
///
/// The shape forwarding has: a value arrives at the head and is handed on.
fn today<T: Clone + Send + Sync + 'static>(initial: T, hops: usize) -> Signal<T> {
    let head = Signal::new(initial.clone());
    let mut current = head.clone();

    for _ in 0..hops {
        let next = Signal::new(initial.clone());
        let sink = next.clone();
        // The clone is not incidental: a subscriber is lent `&T` and `set`
        // wants a `T`, so there is nothing else it could do.
        std::mem::forget(current.subscribe(move |v: &T| sink.set(v.clone())));
        current = next;
    }

    head
}

fn proposed<T: Clone + Send + Sync + 'static>(initial: T, hops: usize) -> ArcSignal<T> {
    let head = ArcSignal::new(initial.clone());
    let mut subs = Arc::clone(&head.subscribers);
    let mut value = Arc::clone(&head.value);

    for _ in 0..hops {
        let next = ArcSignal::<T>::new(initial.clone());
        let sink_value = Arc::clone(&next.value);
        let sink_subs = Arc::clone(&next.subscribers);

        subs.lock().unwrap().push(Arc::new(move |v: &Arc<T>| {
            sink_value.store(Arc::clone(v));
            let callbacks: Vec<_> = sink_subs.lock().unwrap().iter().cloned().collect();
            for cb in callbacks {
                cb(v);
            }
        }));

        subs = next.subscribers;
        value = next.value;
    }

    let _ = value;
    head
}

fn payloads() -> Vec<(&'static str, String)> {
    vec![
        ("16b", "x".repeat(16)),
        ("1k", "x".repeat(1024)),
        ("64k", "x".repeat(64 * 1024)),
    ]
}

/// One write, no subscribers: the floor, which is the `Arc::new` and the swap.
fn a_write_with_nobody_listening(c: &mut Criterion) {
    let mut group = c.benchmark_group("signal/set/no subscribers");

    for (name, payload) in payloads() {
        group.bench_with_input(BenchmarkId::new("today", name), &payload, |b, p| {
            let signal = Signal::new(p.clone());
            b.iter(|| signal.set(black_box(p.clone())));
        });

        group.bench_with_input(BenchmarkId::new("arc travels", name), &payload, |b, p| {
            let signal = ArcSignal::new(p.clone());
            let held = Arc::new(p.clone());
            b.iter(|| signal.set_arc(black_box(Arc::clone(&held))));
        });
    }

    group.finish();
}

/// Three hops, which is an ordinary chain: a field, a derived view, a sink.
fn a_write_travelling_a_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("signal/set/three hops");

    for (name, payload) in payloads() {
        group.bench_with_input(BenchmarkId::new("today", name), &payload, |b, p| {
            let head = today(p.clone(), 3);
            b.iter(|| head.set(black_box(p.clone())));
        });

        group.bench_with_input(BenchmarkId::new("arc travels", name), &payload, |b, p| {
            let head = proposed(p.clone(), 3);
            let held = Arc::new(p.clone());
            b.iter(|| head.set_arc(black_box(Arc::clone(&held))));
        });
    }

    group.finish();
}

/// The allocation on its own, to say what fraction of a write it is.
fn the_allocation_by_itself(c: &mut Criterion) {
    let mut group = c.benchmark_group("signal/the arc alone");

    for (name, payload) in payloads() {
        group.bench_with_input(BenchmarkId::new("clone and box", name), &payload, |b, p| {
            b.iter(|| black_box(Arc::new(p.clone())));
        });

        group.bench_with_input(
            BenchmarkId::new("bump a refcount", name),
            &payload,
            |b, p| {
                let held = Arc::new(p.clone());
                b.iter(|| black_box(Arc::clone(&held)));
            },
        );
    }

    group.finish();
}

/// Reading, which is where the clone actually lives.
///
/// `Signal::get` is `self.value.load().as_ref().clone()`, so every read copies
/// the whole value. A retain-mode UI does that per frame per field, and a
/// forwarding hop does it once per source to build its own value - which is why
/// the chain above costs what it does, and why `set` taking an `Arc` would only
/// have fixed the smaller half.
///
/// Three shapes: the clone as it stands, handing back the `Arc`, and lending
/// the value for the length of a call. The last needs no allocation and no
/// copy at all, and it is what a caller reading one field of a struct actually
/// wants.
fn reading(c: &mut Criterion) {
    let mut group = c.benchmark_group("signal/get");

    for (name, payload) in payloads() {
        let signal = Signal::new(payload.clone());

        group.bench_with_input(BenchmarkId::new("clone out", name), &signal, |b, s| {
            b.iter(|| black_box(s.get()));
        });

        group.bench_with_input(
            BenchmarkId::new("hand back the arc", name),
            &signal,
            |b, s| {
                b.iter(|| black_box(s.value.load_full()));
            },
        );

        group.bench_with_input(BenchmarkId::new("lend it", name), &signal, |b, s| {
            b.iter(|| black_box(s.value.load().len()));
        });
    }

    group.finish();
}

/// What a UI does with a read: look at one part of it.
///
/// Cloning to answer a question about a field is the shape that makes the copy
/// pure waste - the value is dropped on the next line.
fn reading_one_field_of_a_struct(c: &mut Criterion) {
    #[derive(Clone)]
    struct Window {
        _title: String,
        width: u32,
        _rest: Vec<u8>,
    }

    let mut group = c.benchmark_group("signal/get/one field");

    for (name, size) in [("small", 64usize), ("large", 64 * 1024)] {
        let signal = Signal::new(Window {
            _title: "x".repeat(size),
            width: 1280,
            _rest: vec![7; size],
        });

        group.bench_with_input(BenchmarkId::new("clone out", name), &signal, |b, s| {
            b.iter(|| black_box(s.get().width));
        });

        group.bench_with_input(BenchmarkId::new("lend it", name), &signal, |b, s| {
            b.iter(|| black_box(s.value.load().width));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    a_write_with_nobody_listening,
    a_write_travelling_a_chain,
    the_allocation_by_itself,
    reading,
    reading_one_field_of_a_struct
);
criterion_main!(benches);
