//! Measures the cost of putting `Arc<dyn CellBackend<T>>` in front of the
//! current direct `Signal::get()` path.
//!
//! The question: retain-mode UIs call `get()` every frame, so does swapping a
//! direct `ArcSwap::load` for an `Arc` deref + vtable call actually show up?
//!
//! `CellBackend`/`ReactiveCell` here are stand-ins for the proposed types - they
//! model the dispatch shape, nothing else.

use amethystate::Field;
use amethystate_core::Signal;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;

// ---- proposed shape -------------------------------------------------------

trait CellBackend<T>: Send + Sync {
    fn get(&self) -> T;
}

struct SignalCell<T>(Signal<T>);

impl<T: Clone + Send + Sync + 'static> CellBackend<T> for SignalCell<T> {
    fn get(&self) -> T {
        self.0.get()
    }
}

#[derive(Clone)]
struct ReactiveCell<T>(Arc<dyn CellBackend<T>>);

impl<T: Clone + Send + Sync + 'static> ReactiveCell<T> {
    fn new(initial: T) -> Self {
        Self(Arc::new(SignalCell(Signal::new(initial))))
    }

    #[inline]
    fn get(&self) -> T {
        self.0.get()
    }
}

// ---- static dispatch over a closed set of impls ---------------------------

// `Volatile` is never constructed, but it has to exist: with a single variant
// the match folds away and the bench stops measuring dispatch at all.
#[derive(Clone)]
#[allow(dead_code)]
enum CellEnum<T> {
    Volatile(Signal<T>),
    Stored(Signal<T>),
}

impl<T: Clone + Send + Sync + 'static> CellEnum<T> {
    #[inline]
    fn get(&self) -> T {
        match self {
            CellEnum::Volatile(s) => s.get(),
            CellEnum::Stored(s) => s.get(),
        }
    }
}

// ---- cache up front, dispatch only on the write path -----------------------
//
// Every cell already keeps a cached `Signal` - that is what `Signal` is *for*
// once it goes internal. So `get()` can read the cache directly and never
// dispatch at all; only `set()` needs to know where the value goes, and writes
// are user-paced, not frame-paced.

type Writer<T> = Arc<dyn Fn(T) + Send + Sync>;

// `writer` is never read here - only `get()` is benched - but it has to be
// carried so the type's size and clone cost match the real cell.
#[derive(Clone)]
struct CellCached<T> {
    cache: Signal<T>,
    #[allow(dead_code)]
    writer: Option<Writer<T>>,
}

impl<T: Clone + Send + Sync + 'static> CellCached<T> {
    fn stored(initial: T) -> Self {
        let cache = Signal::new(initial);
        let sink = cache.clone();
        Self {
            cache,
            writer: Some(Arc::new(move |v| sink.set(v))),
        }
    }

    #[inline]
    fn get(&self) -> T {
        self.cache.get()
    }
}

// ---- benches --------------------------------------------------------------

fn bench_get<T>(c: &mut Criterion, label: &str, initial: T)
where
    T: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    let mut group = c.benchmark_group(label);

    let signal = Signal::new(initial.clone());
    group.bench_function("signal_direct", |b| {
        b.iter(|| black_box(black_box(&signal).get()))
    });

    let cell = ReactiveCell::new(initial.clone());
    group.bench_function("cell_dyn", |b| b.iter(|| black_box(black_box(&cell).get())));

    // Same vtable call, but through a bare Arc<dyn _> with no newtype wrapper,
    // to confirm the wrapper itself costs nothing.
    let bare: Arc<dyn CellBackend<T>> = Arc::new(SignalCell(Signal::new(initial.clone())));
    group.bench_function("arc_dyn_bare", |b| {
        b.iter(|| black_box(black_box(&bare).get()))
    });

    let en = CellEnum::Stored(Signal::new(initial.clone()));
    group.bench_function("cell_enum", |b| b.iter(|| black_box(black_box(&en).get())));

    let cached = CellCached::stored(initial.clone());
    group.bench_function("cell_cached", |b| {
        b.iter(|| black_box(black_box(&cached).get()))
    });

    // Real primitive, not a stand-in. `Field::get()` is `self.core.get()` ->
    // `signal.get()` and is identical for stored and volatile fields - the store
    // only participates in `set` - so this is the true cost of a field read.
    let field: Field<T> = Field::new_volatile(["bench", "value"], initial);
    group.bench_function("field_real", |b| {
        b.iter(|| black_box(black_box(&field).get()))
    });

    group.finish();
}

fn benches(c: &mut Criterion) {
    // Cheap clone: worst case for relative vtable overhead.
    bench_get(c, "get_u64", 42u64);

    // Allocating clone: realistic for config values, where the clone should
    // swamp any dispatch cost.
    bench_get(c, "get_string", "127.0.0.1:8080".to_string());

    // Bigger payload, still Clone.
    bench_get(c, "get_vec", vec![0u8; 128]);
}

criterion_group!(cell_dispatch, benches);
criterion_main!(cell_dispatch);
