//! What the map's projection should be: a hash map that sorts on demand, or an
//! ordered map that never has to.
//!
//! The contract's order is the name's own, not `K: Ord`, so the variants
//! separate structure, what the key is stored as, and the lock: `dash`, a tree
//! keyed by the name as a `String`, a `Box<str>` or a `SmolStr`, and those
//! under a `Mutex` rather than an `RwLock`.
//!
//! Sizes track the envelope: ten is the common case, a hundred thousand the
//! edge.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use smol_str::SmolStr;
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::hint::black_box;

const FILLS: [usize; 3] = [10, 1_000, 100_000];
const PROBES: usize = 4096;

/// The map keys an application actually has: mostly plain names, and a few
/// holding a separator, which a key holds as itself.
fn names(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            if i % 20 == 0 {
                format!("group.{i:07}")
            } else {
                format!("item-{i:07}")
            }
        })
        .collect()
}

/// Everything the map's projection is asked for, so the same questions can be
/// put to every shape.
trait Cache: Sync {
    fn build(names: &[String]) -> Self;
    fn get(&self, name: &str) -> Option<u64>;
    fn update(&self, name: &str, value: u64);
    fn len(&self) -> usize;
    /// Every key, in the order the contract promises.
    fn keys(&self) -> Vec<String>;
    /// Every entry, in that order, with the values cloned.
    fn entries(&self) -> Vec<(String, u64)>;
    /// The first `n` entries in that order, which is what a table draws.
    fn window(&self, n: usize) -> Vec<(String, u64)>;
}

struct Dash(DashMap<String, u64>);

impl Cache for Dash {
    fn build(names: &[String]) -> Self {
        Self(
            names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), i as u64))
                .collect(),
        )
    }

    fn get(&self, name: &str) -> Option<u64> {
        self.0.get(name).map(|v| *v)
    }

    fn update(&self, name: &str, value: u64) {
        self.0.insert(name.to_owned(), value);
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.0.iter().map(|e| e.key().to_string()).collect();
        keys.sort();
        keys
    }

    fn entries(&self) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64)> = self
            .0
            .iter()
            .map(|e| (e.key().to_string(), *e.value()))
            .collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        entries
    }

    fn window(&self, n: usize) -> Vec<(String, u64)> {
        let mut entries = self.entries();
        entries.truncate(n);
        entries
    }
}

/// What a name can be stored as. All three borrow as `str`; they differ in
/// whether a comparison during a descent leaves the node.
trait StoredKey: Ord + Borrow<str> + for<'a> From<&'a str> + Send + Sync + 'static {}
impl<T: Ord + Borrow<str> + for<'a> From<&'a str> + Send + Sync + 'static> StoredKey for T {}

fn keyed_build<Key: StoredKey>(names: &[String]) -> BTreeMap<Key, (String, u64)> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| (Key::from(n.as_str()), (n.clone(), i as u64)))
        .collect()
}

struct BtreeRw<Key: StoredKey>(RwLock<BTreeMap<Key, (String, u64)>>);

impl<Key: StoredKey> Cache for BtreeRw<Key> {
    fn build(names: &[String]) -> Self {
        Self(RwLock::new(keyed_build(names)))
    }

    fn get(&self, name: &str) -> Option<u64> {
        self.0.read().get(name).map(|(_, v)| *v)
    }

    fn update(&self, name: &str, value: u64) {
        self.0
            .write()
            .insert(Key::from(name), (name.to_owned(), value));
    }

    fn len(&self) -> usize {
        self.0.read().len()
    }

    fn keys(&self) -> Vec<String> {
        self.0.read().values().map(|(k, _)| k.clone()).collect()
    }

    fn entries(&self) -> Vec<(String, u64)> {
        self.0
            .read()
            .values()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    fn window(&self, n: usize) -> Vec<(String, u64)> {
        self.0
            .read()
            .values()
            .take(n)
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }
}

struct BtreeMutex<Key: StoredKey>(Mutex<BTreeMap<Key, (String, u64)>>);

impl<Key: StoredKey> Cache for BtreeMutex<Key> {
    fn build(names: &[String]) -> Self {
        Self(Mutex::new(keyed_build(names)))
    }

    fn get(&self, name: &str) -> Option<u64> {
        self.0.lock().get(name).map(|(_, v)| *v)
    }

    fn update(&self, name: &str, value: u64) {
        self.0
            .lock()
            .insert(Key::from(name), (name.to_owned(), value));
    }

    fn len(&self) -> usize {
        self.0.lock().len()
    }

    fn keys(&self) -> Vec<String> {
        self.0.lock().values().map(|(k, _)| k.clone()).collect()
    }

    fn entries(&self) -> Vec<(String, u64)> {
        self.0
            .lock()
            .values()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    fn window(&self, n: usize) -> Vec<(String, u64)> {
        self.0
            .lock()
            .values()
            .take(n)
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }
}

/// A fixed shuffle, so every shape is asked for the same keys in the same order
/// and no prefetcher can follow it.
fn probes(fill: usize) -> Vec<usize> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    (0..PROBES)
        .map(|_| (next() % fill as u64) as usize)
        .collect()
}

fn point_ops<C: Cache>(c: &mut Criterion, shape: &str) {
    let mut group = c.benchmark_group("map_cache/point");
    group.throughput(Throughput::Elements(PROBES as u64));

    for fill in FILLS {
        let names = names(fill);
        let cache = C::build(&names);
        let order = probes(fill);

        group.bench_with_input(
            BenchmarkId::new(format!("get/{shape}"), fill),
            &fill,
            |b, _| {
                b.iter(|| {
                    let mut acc = 0u64;
                    for &i in &order {
                        acc ^= cache.get(&names[i]).unwrap_or(0);
                    }
                    acc
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new(format!("update/{shape}"), fill),
            &fill,
            |b, _| {
                b.iter(|| {
                    for &i in &order {
                        cache.update(&names[i], i as u64);
                    }
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new(format!("len/{shape}"), fill),
            &fill,
            |b, _| {
                b.iter(|| {
                    let mut acc = 0usize;
                    for _ in 0..PROBES {
                        acc ^= cache.len();
                    }
                    acc
                })
            },
        );
    }

    group.finish();
}

fn ordered_ops<C: Cache>(c: &mut Criterion, shape: &str) {
    let mut group = c.benchmark_group("map_cache/ordered");
    group.sample_size(20);

    for fill in FILLS {
        let names = names(fill);
        let cache = C::build(&names);

        group.throughput(Throughput::Elements(fill as u64));
        group.bench_with_input(
            BenchmarkId::new(format!("keys/{shape}"), fill),
            &fill,
            |b, _| b.iter(|| black_box(cache.keys().len())),
        );
        group.bench_with_input(
            BenchmarkId::new(format!("entries/{shape}"), fill),
            &fill,
            |b, _| b.iter(|| black_box(cache.entries().len())),
        );

        group.throughput(Throughput::Elements(50));
        group.bench_with_input(
            BenchmarkId::new(format!("window50/{shape}"), fill),
            &fill,
            |b, _| b.iter(|| black_box(cache.window(50).len())),
        );
    }

    group.finish();
}

/// The patterns a GUI actually produces: many widgets reading at once, and the
/// same with one of them writing, where the lock matters as much as the
/// structure.
fn concurrent_ops<C: Cache>(c: &mut Criterion, shape: &str) {
    const THREADS: usize = 4;
    const PER_THREAD: usize = 4096;

    let mut group = c.benchmark_group("map_cache/concurrent");
    group.throughput(Throughput::Elements((THREADS * PER_THREAD) as u64));

    for fill in [1_000usize, 100_000] {
        let names = names(fill);
        let cache = C::build(&names);
        let order = probes(fill);

        group.bench_with_input(
            BenchmarkId::new(format!("readers/{shape}"), fill),
            &fill,
            |b, _| {
                b.iter(|| {
                    (0..THREADS)
                        .into_par_iter()
                        .map(|t| {
                            let mut acc = 0u64;
                            for k in 0..PER_THREAD {
                                let i = order[(k + t * 97) % order.len()];
                                acc ^= cache.get(&names[i]).unwrap_or(0);
                            }
                            acc
                        })
                        .reduce(|| 0, |a, b| a ^ b)
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new(format!("readers+writer/{shape}"), fill),
            &fill,
            |b, _| {
                b.iter(|| {
                    (0..THREADS)
                        .into_par_iter()
                        .map(|t| {
                            let mut acc = 0u64;
                            for k in 0..PER_THREAD {
                                let i = order[(k + t * 97) % order.len()];
                                if t == 0 {
                                    cache.update(&names[i], k as u64);
                                } else {
                                    acc ^= cache.get(&names[i]).unwrap_or(0);
                                }
                            }
                            acc
                        })
                        .reduce(|| 0, |a, b| a ^ b)
                })
            },
        );
    }

    group.finish();
}

fn benches(c: &mut Criterion) {
    point_ops::<Dash>(c, "dash");
    point_ops::<BtreeRw<String>>(c, "btree/string");
    point_ops::<BtreeRw<Box<str>>>(c, "btree/boxed");
    point_ops::<BtreeRw<SmolStr>>(c, "btree/smol");
    point_ops::<BtreeMutex<String>>(c, "btree/string+mutex");
    point_ops::<BtreeMutex<SmolStr>>(c, "btree/smol+mutex");

    ordered_ops::<Dash>(c, "dash");
    ordered_ops::<BtreeRw<String>>(c, "btree/string");
    ordered_ops::<BtreeRw<Box<str>>>(c, "btree/boxed");
    ordered_ops::<BtreeRw<SmolStr>>(c, "btree/smol");

    concurrent_ops::<Dash>(c, "dash");
    concurrent_ops::<BtreeRw<String>>(c, "btree/string");
    concurrent_ops::<BtreeRw<Box<str>>>(c, "btree/boxed");
    concurrent_ops::<BtreeRw<SmolStr>>(c, "btree/smol");
    concurrent_ops::<BtreeMutex<SmolStr>>(c, "btree/smol+mutex");
}

criterion_group!(map_cache_shape, benches);
criterion_main!(map_cache_shape);
