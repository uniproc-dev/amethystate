//! What the map's projection should be: a hash map that sorts on demand, or an
//! ordered map that never has to.
//!
//! The contract's order is the escaped name's, not `K: Ord`, so the variants
//! separate three questions at once - structure, key representation, and lock:
//! `dash`, a tree keyed by the name with `Ord` through `cmp_names`, a tree
//! keyed by the escaped name, and that one under a `Mutex`.
//!
//! Sizes track the envelope: ten is the common case, a hundred thousand the
//! edge.

use amethystate_core::path::cmp_names;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use smol_str::SmolStr;
use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hint::black_box;

const FILLS: [usize; 3] = [10, 1_000, 100_000];
const PROBES: usize = 4096;
const SEPARATOR: char = '.';
const ESCAPE: char = '\\';

/// The map keys an application actually has: mostly plain names, and a few
/// holding the separator, so escaping is not free by luck.
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

fn escape(name: &str) -> Cow<'_, str> {
    if !name.contains([SEPARATOR, ESCAPE]) {
        return Cow::Borrowed(name);
    }

    let mut out = String::with_capacity(name.len() + 4);
    for ch in name.chars() {
        if ch == SEPARATOR || ch == ESCAPE {
            out.push(ESCAPE);
        }
        out.push(ch);
    }
    Cow::Owned(out)
}

/// A name ordered the way the store orders the key it becomes, without holding
/// the escaped form.
#[derive(PartialEq, Eq)]
struct Name(String);

impl Ord for Name {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_names(&self.0, &other.0)
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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
        keys.sort_by(|a, b| cmp_names(a, b));
        keys
    }

    fn entries(&self) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64)> = self
            .0
            .iter()
            .map(|e| (e.key().to_string(), *e.value()))
            .collect();
        entries.sort_by(|(a, _), (b, _)| cmp_names(a, b));
        entries
    }

    fn window(&self, n: usize) -> Vec<(String, u64)> {
        let mut entries = self.entries();
        entries.truncate(n);
        entries
    }
}

struct BtreeName(RwLock<BTreeMap<Name, u64>>);

impl Cache for BtreeName {
    fn build(names: &[String]) -> Self {
        Self(RwLock::new(
            names
                .iter()
                .enumerate()
                .map(|(i, n)| (Name(n.clone()), i as u64))
                .collect(),
        ))
    }

    fn get(&self, name: &str) -> Option<u64> {
        let probe = Name(name.to_owned());
        self.0.read().get(&probe).copied()
    }

    fn update(&self, name: &str, value: u64) {
        self.0.write().insert(Name(name.to_owned()), value);
    }

    fn len(&self) -> usize {
        self.0.read().len()
    }

    fn keys(&self) -> Vec<String> {
        self.0.read().keys().map(|k| k.0.clone()).collect()
    }

    fn entries(&self) -> Vec<(String, u64)> {
        self.0
            .read()
            .iter()
            .map(|(k, v)| (k.0.clone(), *v))
            .collect()
    }

    fn window(&self, n: usize) -> Vec<(String, u64)> {
        self.0
            .read()
            .iter()
            .take(n)
            .map(|(k, v)| (k.0.clone(), *v))
            .collect()
    }
}

/// What the escaped form can be stored as. All three borrow as `str`; they
/// differ in whether a comparison during a descent leaves the node.
trait EscapedKey: Ord + Borrow<str> + for<'a> From<&'a str> + Send + Sync + 'static {}
impl<T: Ord + Borrow<str> + for<'a> From<&'a str> + Send + Sync + 'static> EscapedKey for T {}

fn escaped_build<Key: EscapedKey>(names: &[String]) -> BTreeMap<Key, (String, u64)> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| (Key::from(escape(n).as_ref()), (n.clone(), i as u64)))
        .collect()
}

struct BtreeEscapedRw<Key: EscapedKey>(RwLock<BTreeMap<Key, (String, u64)>>);

impl<Key: EscapedKey> Cache for BtreeEscapedRw<Key> {
    fn build(names: &[String]) -> Self {
        Self(RwLock::new(escaped_build(names)))
    }

    fn get(&self, name: &str) -> Option<u64> {
        self.0.read().get(escape(name).as_ref()).map(|(_, v)| *v)
    }

    fn update(&self, name: &str, value: u64) {
        self.0
            .write()
            .insert(Key::from(escape(name).as_ref()), (name.to_owned(), value));
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

struct BtreeEscapedMutex<Key: EscapedKey>(Mutex<BTreeMap<Key, (String, u64)>>);

impl<Key: EscapedKey> Cache for BtreeEscapedMutex<Key> {
    fn build(names: &[String]) -> Self {
        Self(Mutex::new(escaped_build(names)))
    }

    fn get(&self, name: &str) -> Option<u64> {
        self.0.lock().get(escape(name).as_ref()).map(|(_, v)| *v)
    }

    fn update(&self, name: &str, value: u64) {
        self.0
            .lock()
            .insert(Key::from(escape(name).as_ref()), (name.to_owned(), value));
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
    point_ops::<BtreeName>(c, "btree/name");
    point_ops::<BtreeEscapedRw<String>>(c, "btree/escaped");
    point_ops::<BtreeEscapedRw<Box<str>>>(c, "btree/boxed");
    point_ops::<BtreeEscapedRw<SmolStr>>(c, "btree/smol");
    point_ops::<BtreeEscapedMutex<String>>(c, "btree/escaped+mutex");
    point_ops::<BtreeEscapedMutex<SmolStr>>(c, "btree/smol+mutex");

    ordered_ops::<Dash>(c, "dash");
    ordered_ops::<BtreeName>(c, "btree/name");
    ordered_ops::<BtreeEscapedRw<String>>(c, "btree/escaped");
    ordered_ops::<BtreeEscapedRw<Box<str>>>(c, "btree/boxed");
    ordered_ops::<BtreeEscapedRw<SmolStr>>(c, "btree/smol");

    concurrent_ops::<Dash>(c, "dash");
    concurrent_ops::<BtreeName>(c, "btree/name");
    concurrent_ops::<BtreeEscapedRw<String>>(c, "btree/escaped");
    concurrent_ops::<BtreeEscapedRw<Box<str>>>(c, "btree/boxed");
    concurrent_ops::<BtreeEscapedRw<SmolStr>>(c, "btree/smol");
    concurrent_ops::<BtreeEscapedMutex<SmolStr>>(c, "btree/smol+mutex");
}

criterion_group!(map_cache_shape, benches);
criterion_main!(map_cache_shape);
