//! What a snapshot costs against the read guard it replaced.
//!
//! `MapCache` hands a walk its own version and holds nothing, so writing
//! during a walk stops deadlocking. It reached that shape on these numbers,
//! and it keeps them because the alternative is what the numbers are against:
//! a `RwLock<BTreeMap>` holding its guard for the life of the walk. Four
//! shapes over the same data:
//!
//! - `rwlock+btreemap`, the shape this replaced;
//! - `arcswap+clone`, a read that is `load()` and a write that copies the whole
//!   map;
//! - `arcswap+im`, the same read with `im::OrdMap` doing the write by path
//!   copying;
//! - `arcswap+rpds`, the same with `rpds::RedBlackTreeMapSync`.
//!
//! `K` is `String` and `V` is `u64`, so every entry carries one heap allocation
//! of its own. That is what makes the copying write expensive and it is what a
//! real projection holds, so it stays.
//!
//! Insert and remove are timed with `iter_custom` over a pool of spare keys:
//! the pool is inserted under the clock and removed off it, or the other way
//! round, so each timed iteration is one real insert or one real remove and the
//! map's size drifts by at most one percent. Overwrite, `get` and the walks need
//! no such care and use `iter`.
//!
//! Run one shape per process - `cargo bench --bench map_snapshot_bench -- rwlock`
//! and then once per `arcswap.clone`, `arcswap.im`, `arcswap.rpds`. Running all
//! four together makes the walks unreadable: the write benchmarks leave the heap
//! fragmented, and a shape measured after three others has worse locality than
//! the one measured first. Two shapes here hold the same `BTreeMap`, so the gap
//! between `rwlock+btreemap` and `arcswap+clone` on a walk is the measurement's
//! own error - it read up to two-fold in one process and under ten percent in
//! four.
//!
//! Run each shape at least twice and compare with `--save-baseline` and
//! `--baseline`. Criterion's interval describes variance inside one run and says
//! nothing about a machine that got busy halfway through; two runs agreeing is
//! what carries the number. Three campaigns here each had one shape disturbed
//! enough to move a write by half, and in every case the disturbed run was the
//! one that disagreed with the other two rather than the one with a wide
//! interval. Before trusting a table, read it across sizes: a hundred thousand
//! costing about what ten thousand costs, or a near-linear operation that is
//! not, is a disturbed run saying so in arithmetic.

use arc_swap::ArcSwap;
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main};
use im::OrdMap;
use parking_lot::{ArcRwLockReadGuard, RawRwLock, RwLock};
use rpds::RedBlackTreeMapSync;
use smol_str::SmolStr;
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Entry = (String, u64);

const SIZES: [usize; 3] = [1_000, 10_000, 100_000];
const PROBES: usize = 64;

fn name(i: usize) -> SmolStr {
    SmolStr::new(format!("item-{i:07}"))
}

fn written() -> Entry {
    ("payload-value".to_string(), 42)
}

/// The data every shape is built from, and the keys the operations use.
///
/// Present keys are even, spare keys odd and spread across the range, so an
/// insert lands in the middle of the order rather than always at its edge.
struct Base {
    entries: Vec<(SmolStr, Entry)>,
    spares: Vec<SmolStr>,
    probes: Vec<SmolStr>,
}

fn base(n: usize) -> Base {
    let entries = (0..n)
        .map(|i| {
            let key = name(i * 2);
            (key.clone(), (key.to_string(), i as u64))
        })
        .collect();

    let pool = (n / 100).clamp(1, 1024);
    let spares = (0..pool).map(|j| name(j * n / pool * 2 + 1)).collect();
    let probes = (0..PROBES).map(|j| name(j * n / PROBES * 2)).collect();

    Base {
        entries,
        spares,
        probes,
    }
}

/// Everything the projection is asked for, put to every shape the same way.
trait Shape {
    const NAME: &'static str;

    /// What a walk would be handed and would hold onto.
    type Snap;

    fn build(base: &Base) -> Self;
    fn write(&self, key: &SmolStr, value: &Entry);
    fn remove(&self, key: &SmolStr);
    fn get(&self, key: &SmolStr) -> Option<u64>;
    fn snapshot(&self) -> Self::Snap;

    /// `entries()`: every entry cloned out, in order.
    fn walk_entries(&self) -> usize;

    /// `keys()`: every key cloned out, in order.
    fn walk_keys(&self) -> usize;

    /// `view()`: the same traversal borrowing rather than cloning, which is the
    /// only one of the three that shows the structure instead of the payload.
    fn walk_borrowed(&self) -> u64;
}

struct Guarded(Arc<RwLock<BTreeMap<SmolStr, Entry>>>);

impl Shape for Guarded {
    const NAME: &'static str = "rwlock+btreemap";

    type Snap = ArcRwLockReadGuard<RawRwLock, BTreeMap<SmolStr, Entry>>;

    fn build(base: &Base) -> Self {
        Self(Arc::new(RwLock::new(
            base.entries.iter().cloned().collect(),
        )))
    }

    fn write(&self, key: &SmolStr, value: &Entry) {
        self.0.write().insert(key.clone(), value.clone());
    }

    fn remove(&self, key: &SmolStr) {
        self.0.write().remove(key);
    }

    fn get(&self, key: &SmolStr) -> Option<u64> {
        self.0.read().get(key).map(|(_, v)| *v)
    }

    fn snapshot(&self) -> Self::Snap {
        self.0.read_arc()
    }

    fn walk_entries(&self) -> usize {
        let held = self.0.read_arc();
        let mut acc = 0;
        for entry in held.values() {
            let taken: Entry = entry.clone();
            acc += black_box(&taken).0.len();
        }
        acc
    }

    fn walk_keys(&self) -> usize {
        let held = self.0.read_arc();
        let mut acc = 0;
        for (key, _) in held.values() {
            let taken: String = key.clone();
            acc += black_box(&taken).len();
        }
        acc
    }

    fn walk_borrowed(&self) -> u64 {
        let held = self.0.read();
        held.values().map(|(_, v)| *v).sum()
    }
}

struct Cloned(ArcSwap<BTreeMap<SmolStr, Entry>>);

impl Shape for Cloned {
    const NAME: &'static str = "arcswap+clone";

    type Snap = Arc<BTreeMap<SmolStr, Entry>>;

    fn build(base: &Base) -> Self {
        Self(ArcSwap::from_pointee(
            base.entries.iter().cloned().collect(),
        ))
    }

    fn write(&self, key: &SmolStr, value: &Entry) {
        self.0.rcu(|current| {
            let mut next = (**current).clone();
            next.insert(key.clone(), value.clone());
            next
        });
    }

    fn remove(&self, key: &SmolStr) {
        self.0.rcu(|current| {
            let mut next = (**current).clone();
            next.remove(key);
            next
        });
    }

    fn get(&self, key: &SmolStr) -> Option<u64> {
        self.0.load().get(key).map(|(_, v)| *v)
    }

    fn snapshot(&self) -> Self::Snap {
        self.0.load_full()
    }

    fn walk_entries(&self) -> usize {
        let held = self.0.load_full();
        let mut acc = 0;
        for entry in held.values() {
            let taken: Entry = entry.clone();
            acc += black_box(&taken).0.len();
        }
        acc
    }

    fn walk_keys(&self) -> usize {
        let held = self.0.load_full();
        let mut acc = 0;
        for (key, _) in held.values() {
            let taken: String = key.clone();
            acc += black_box(&taken).len();
        }
        acc
    }

    fn walk_borrowed(&self) -> u64 {
        let held = self.0.load();
        held.values().map(|(_, v)| *v).sum()
    }
}

struct ImOrd(ArcSwap<OrdMap<SmolStr, Entry>>);

impl Shape for ImOrd {
    const NAME: &'static str = "arcswap+im";

    type Snap = Arc<OrdMap<SmolStr, Entry>>;

    fn build(base: &Base) -> Self {
        let mut map = OrdMap::new();
        for (key, entry) in &base.entries {
            map.insert(key.clone(), entry.clone());
        }
        Self(ArcSwap::from_pointee(map))
    }

    fn write(&self, key: &SmolStr, value: &Entry) {
        self.0.rcu(|current| {
            let mut next = (**current).clone();
            next.insert(key.clone(), value.clone());
            next
        });
    }

    fn remove(&self, key: &SmolStr) {
        self.0.rcu(|current| {
            let mut next = (**current).clone();
            next.remove(key);
            next
        });
    }

    fn get(&self, key: &SmolStr) -> Option<u64> {
        self.0.load().get(key).map(|(_, v)| *v)
    }

    fn snapshot(&self) -> Self::Snap {
        self.0.load_full()
    }

    fn walk_entries(&self) -> usize {
        let held = self.0.load_full();
        let mut acc = 0;
        for entry in held.values() {
            let taken: Entry = entry.clone();
            acc += black_box(&taken).0.len();
        }
        acc
    }

    fn walk_keys(&self) -> usize {
        let held = self.0.load_full();
        let mut acc = 0;
        for (key, _) in held.values() {
            let taken: String = key.clone();
            acc += black_box(&taken).len();
        }
        acc
    }

    fn walk_borrowed(&self) -> u64 {
        let held = self.0.load();
        held.values().map(|(_, v)| *v).sum()
    }
}

struct Rpds(ArcSwap<RedBlackTreeMapSync<SmolStr, Entry>>);

impl Shape for Rpds {
    const NAME: &'static str = "arcswap+rpds";

    type Snap = Arc<RedBlackTreeMapSync<SmolStr, Entry>>;

    fn build(base: &Base) -> Self {
        let mut map = RedBlackTreeMapSync::new_sync();
        for (key, entry) in &base.entries {
            map = map.insert(key.clone(), entry.clone());
        }
        Self(ArcSwap::from_pointee(map))
    }

    fn write(&self, key: &SmolStr, value: &Entry) {
        self.0
            .rcu(|current| current.insert(key.clone(), value.clone()));
    }

    fn remove(&self, key: &SmolStr) {
        self.0.rcu(|current| current.remove(key));
    }

    fn get(&self, key: &SmolStr) -> Option<u64> {
        self.0.load().get(key).map(|(_, v)| *v)
    }

    fn snapshot(&self) -> Self::Snap {
        self.0.load_full()
    }

    fn walk_entries(&self) -> usize {
        let held = self.0.load_full();
        let mut acc = 0;
        for entry in held.values() {
            let taken: Entry = entry.clone();
            acc += black_box(&taken).0.len();
        }
        acc
    }

    fn walk_keys(&self) -> usize {
        let held = self.0.load_full();
        let mut acc = 0;
        for (key, _) in held.values() {
            let taken: String = key.clone();
            acc += black_box(&taken).len();
        }
        acc
    }

    fn walk_borrowed(&self) -> u64 {
        let held = self.0.load();
        held.values().map(|(_, v)| *v).sum()
    }
}

fn tune(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_millis(1500));
}

fn add_write<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);
        let value = written();
        let mut at = 0usize;

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter(|| {
                at = (at + 1) % base.probes.len();
                shape.write(black_box(&base.probes[at]), black_box(&value));
            })
        });
    }
}

fn add_insert<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);
        let value = written();

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut done = 0u64;

                while done < iters {
                    let take = base.spares.len().min((iters - done) as usize);
                    let batch = &base.spares[..take];

                    let start = Instant::now();
                    for key in batch {
                        shape.write(black_box(key), black_box(&value));
                    }
                    total += start.elapsed();

                    for key in batch {
                        shape.remove(key);
                    }
                    done += take as u64;
                }

                total
            })
        });
    }
}

fn add_remove<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);
        let value = written();

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut done = 0u64;

                while done < iters {
                    let take = base.spares.len().min((iters - done) as usize);
                    let batch = &base.spares[..take];

                    for key in batch {
                        shape.write(key, &value);
                    }

                    let start = Instant::now();
                    for key in batch {
                        shape.remove(black_box(key));
                    }
                    total += start.elapsed();

                    done += take as u64;
                }

                total
            })
        });
    }
}

fn add_get<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);
        let mut at = 0usize;

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter(|| {
                at = (at + 1) % base.probes.len();
                black_box(shape.get(black_box(&base.probes[at])))
            })
        });
    }
}

fn add_snapshot<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter(|| black_box(shape.snapshot()))
        });
    }
}

fn add_entries<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter(|| black_box(shape.walk_entries()))
        });
    }
}

fn add_keys<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter(|| black_box(shape.walk_keys()))
        });
    }
}

fn add_borrowed<S: Shape>(group: &mut BenchmarkGroup<'_, WallTime>, bases: &[(usize, Base)]) {
    for (size, base) in bases {
        let shape = S::build(base);

        group.bench_function(BenchmarkId::new(S::NAME, size), |b| {
            b.iter(|| black_box(shape.walk_borrowed()))
        });
    }
}

macro_rules! every_shape {
    ($add:ident, $group:expr, $bases:expr) => {{
        $add::<Guarded>($group, $bases);
        $add::<Cloned>($group, $bases);
        $add::<ImOrd>($group, $bases);
        $add::<Rpds>($group, $bases);
    }};
}

macro_rules! operation {
    ($fname:ident, $label:literal, $add:ident) => {
        fn $fname(c: &mut Criterion) {
            let bases: Vec<(usize, Base)> = SIZES.iter().map(|&n| (n, base(n))).collect();
            let mut group = c.benchmark_group($label);
            tune(&mut group);
            every_shape!($add, &mut group, &bases);
            group.finish();
        }
    };
}

operation!(writing_one_entry, "map snapshot/write existing", add_write);
operation!(inserting_one_entry, "map snapshot/insert new", add_insert);
operation!(removing_one_entry, "map snapshot/remove", add_remove);
operation!(one_get, "map snapshot/get", add_get);
operation!(taking_a_read, "map snapshot/take a read", add_snapshot);
operation!(walking_entries, "map snapshot/walk entries", add_entries);
operation!(walking_keys, "map snapshot/walk keys", add_keys);
operation!(walking_borrowed, "map snapshot/walk borrowed", add_borrowed);

criterion_group!(
    benches,
    writing_one_entry,
    inserting_one_entry,
    removing_one_entry,
    one_get,
    taking_a_read,
    walking_entries,
    walking_keys,
    walking_borrowed
);
criterion_main!(benches);
