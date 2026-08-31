//! Where the time goes in `ReactiveMap`.
//!
//! The questions an app author actually asks: does a write get slower as the
//! map fills up, what does `len()` cost when it has to scan, does `entries()`
//! pay for values it never yields, and what do subscribers and durability add
//! on top of a plain write.
//!
//! Every group builds its own store under its own path - redb holds an
//! exclusive lock for the life of the process - and uses a debounce far longer
//! than any run, so a "write" bench measures the write and not a flush.

#![allow(clippy::unit_arg)]

use amethystate::store::StoreBackend;
use amethystate::uuid::Uuid;
use amethystate::{ReactiveMap, Store, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::primitives::map_core::ReactiveMapCore;
use amethystate_core::test_utils::TempPath;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

type Map = ReactiveMap<String, u64>;

/// `ReactiveMap` as it is: the parts behind one `Arc`.
#[derive(Clone)]
struct Wrapped {
    inner: Arc<Parts>,
}

struct Parts {
    core: ReactiveMapCore<String, u64>,
    path: StorePath,
    instance_id: Uuid,
    _store: Store,
    _store_sub: Arc<()>,
}

/// The same parts without it. `Arc<()>` stands in for the store subscription,
/// which cannot be built from a bench and clones like any other `Arc`.
#[derive(Clone)]
struct Flat {
    core: ReactiveMapCore<String, u64>,
    path: StorePath,
    instance_id: Uuid,
    store: Store,
    store_sub: Arc<()>,
}

/// A stored value with more than one field in it, which is what a declared
/// struct actually is.
///
/// A `u64` decodes in about eleven nanoseconds, so measuring parallelism
/// against one measures the handing out and nothing else. This is the smallest
/// shape that is not that.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default, PartialEq)]
struct Row {
    title: String,
    host: String,
    port: u16,
    done: bool,
    updated_at: i64,
}

const SIZES: [usize; 3] = [10, 1_000, 10_000];

/// The sizes for the two groups that answer "what does it cost to have this
/// much data", rather than "what does one operation cost".
///
/// It runs two decades further than [`SIZES`] because the answer is a design
/// input rather than a curiosity: this library is aimed at a million records
/// at the outside, and where opening stops fitting in a frame is what decides
/// whether a collection can be built on the rendering thread. Extrapolating
/// from ten thousand was an argument; these are measurements.
///
/// Only the scan and the open take it. The per-operation groups measure a
/// single insert or a single read, and those do not become more interesting
/// with a larger map behind them - they become slower to set up.
const OPEN_SIZES: [usize; 5] = [10, 1_000, 10_000, 100_000, 1_000_000];

/// A store and the guard that removes its files.
///
/// The guard is first in every tuple these helpers return, so it is declared
/// first at the call site and dropped last: the store closes, and only then is
/// the file swept. A million entries is 64 MiB of redb, and a bench run that
/// keeps them leaves that much behind per group.
fn store(tag: &str) -> (TempPath, Store) {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path())
        .disk(|d| d.debounce(Duration::from_secs(100)))
        .build()
        .unwrap();
    (path, store)
}

fn key(i: usize) -> String {
    format!("k{i:07}")
}

fn populated(tag: &str, n: usize) -> (TempPath, Store, Map) {
    let (path, store) = store(tag);
    let map = store.kv().map::<String, u64>("bench").unwrap();
    for i in 0..n {
        map.insert(key(i), &(i as u64)).unwrap();
    }
    (path, store, map)
}

/// The same, with everything committed and the buffer empty.
///
/// Which is the other half of the question and the one an application asks:
/// [`populated`] leaves every entry pending, so a scan over it folds the write
/// buffer and never reaches the engine. Starting up is the reverse - the file
/// holds the data and nothing is buffered - and the two are different code
/// with different costs. A figure from one of them does not answer for the
/// other.
fn committed(tag: &str, n: usize) -> (TempPath, Store, Map) {
    let (path, store, map) = populated(tag, n);
    StoreBackend::save_now(&store).unwrap();
    (path, store, map)
}

/// Committed, with `pending` of its entries written again and spread across the
/// range: the state between flushes, and the only one where the merge merges.
fn half_flushed(tag: &str, n: usize, pending: usize) -> (TempPath, Store, Map) {
    let (path, store, map) = committed(tag, n);
    let stride = (n / pending.max(1)).max(1);
    for i in (0..n).step_by(stride).take(pending) {
        map.insert(key(i), &7).unwrap();
    }
    (path, store, map)
}

fn bench_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_write_vs_size");

    for n in SIZES {
        let (_tmp, _store, map) = populated("map-insert", n);
        let mut next = n;
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::new("insert_new", n), &n, |b, _| {
            b.iter(|| {
                black_box(map.insert(key(next), &1).unwrap());
                next += 1;
            })
        });

        let (_tmp, _store, map) = populated("map-replace", n.max(1));
        group.bench_with_input(BenchmarkId::new("insert_existing", n), &n, |b, _| {
            b.iter(|| black_box(map.insert(key(0), &7).unwrap()))
        });

        let (_tmp, _store, map) = populated("map-update", n.max(1));
        group.bench_with_input(BenchmarkId::new("update", n), &n, |b, _| {
            b.iter(|| black_box(map.update(&key(0), &7).unwrap()))
        });

        let (_tmp, _store, map) = populated("map-modify", n.max(1));
        group.bench_with_input(BenchmarkId::new("modify", n), &n, |b, _| {
            b.iter(|| black_box(map.modify(&key(0), |v| *v += 1).unwrap()))
        });
    }

    group.finish();
}

fn bench_len(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_len_vs_size");

    for n in SIZES {
        let (_tmp, _store, map) = populated("map-len", n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("len", n), &n, |b, _| {
            b.iter(|| black_box(map.len()))
        });
        group.bench_with_input(BenchmarkId::new("is_empty", n), &n, |b, _| {
            b.iter(|| black_box(map.is_empty()))
        });
    }

    group.finish();
}

fn bench_scans(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_scan_vs_size");

    for n in SIZES {
        let (_tmp, _store, map) = populated("map-scan", n);
        group.throughput(Throughput::Elements(n as u64));

        group.bench_with_input(BenchmarkId::new("entries_all", n), &n, |b, _| {
            b.iter(|| black_box(map.entries().count()))
        });
        group.bench_with_input(BenchmarkId::new("keys", n), &n, |b, _| {
            b.iter(|| black_box(map.keys().count()))
        });
        group.bench_with_input(BenchmarkId::new("entries_take1", n), &n, |b, _| {
            b.iter(|| black_box(map.entries().take(1).count()))
        });
    }

    group.finish();
}

/// What a table costs when the values are not numbers and the render wants a
/// window.
///
/// `entries()` clones every key twice - once to a `String` to sort by, once as
/// the key - clones every value, collects the lot and sorts it, per call. A
/// function drawing rows 7 to 34 pays for all of them, every frame, and
/// `RFC-reactive-table.md` opens with that. This is what "all of them" costs
/// once a value is a row rather than a counter.
///
/// `keys()` is the floor: the same collect and sort with no value cloned. The
/// difference between the two is what the values cost, and it is the part a
/// window would not have to pay.
fn bench_windowed_reads(c: &mut Criterion) {
    /// Sized like a row somebody would render: a few fields and some text.
    fn row(i: usize) -> String {
        format!("{i:07}|{}|{}", "name".repeat(8), "x".repeat(200))
    }

    let mut group = c.benchmark_group("map_window_vs_size");

    for n in [1_000usize, 10_000] {
        let (_tmp, _store) = store(&format!("map-window-{n}"));
        let (_tmp2, _store2, light) = populated(&format!("map-window-light-{n}"), n);

        let path = TempPath::new(&format!("map-window-heavy-{n}"));
        let heavy_store = StoreBuilder::new(path.path())
            .disk(|d| d.debounce(Duration::from_secs(100)))
            .build()
            .unwrap();
        let heavy = heavy_store.kv().map::<String, String>("bench").unwrap();
        for i in 0..n {
            heavy.insert(key(i), &row(i)).unwrap();
        }

        group.throughput(Throughput::Elements(n as u64));

        // The window a table draws: twenty-seven rows out of however many.
        group.bench_with_input(BenchmarkId::new("u64/window of 27", n), &n, |b, _| {
            b.iter(|| black_box(light.entries().skip(7).take(27).count()))
        });
        group.bench_with_input(BenchmarkId::new("u64/every row", n), &n, |b, _| {
            b.iter(|| black_box(light.entries().count()))
        });

        group.bench_with_input(BenchmarkId::new("row/window of 27", n), &n, |b, _| {
            b.iter(|| black_box(heavy.entries().skip(7).take(27).count()))
        });
        group.bench_with_input(BenchmarkId::new("row/every row", n), &n, |b, _| {
            b.iter(|| black_box(heavy.entries().count()))
        });

        // The floor: sorting the same keys with no value touched.
        group.bench_with_input(BenchmarkId::new("row/keys only", n), &n, |b, _| {
            b.iter(|| black_box(heavy.keys().count()))
        });

        // What a windowed read could be: the order settled once, then only the
        // rows asked for are fetched. A stand-in for the shape, not a proposal
        // for the API.
        let ordered: Vec<String> = heavy.keys().collect();
        group.bench_with_input(BenchmarkId::new("row/window, order kept", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    ordered
                        .iter()
                        .skip(7)
                        .take(27)
                        .filter_map(|k| heavy.get(k))
                        .count(),
                )
            })
        });
    }

    group.finish();
}

/// Whether the handle needs the `Arc` it is behind: one refcount bump on a
/// clone against a pointer hop on every read, with the handle hot.
fn bench_handle_shape(c: &mut Criterion) {
    let (_tmp, store) = store("map-handle-shape");
    let path = StorePath::from_segments(["bench", "handle"]);
    let id = Uuid::new_v4();

    let wrapped = Wrapped {
        inner: Arc::new(Parts {
            core: ReactiveMapCore::new(),
            path: path.clone(),
            instance_id: id,
            _store: store.clone(),
            _store_sub: Arc::new(()),
        }),
    };
    let flat = Flat {
        core: ReactiveMapCore::new(),
        path,
        instance_id: id,
        store: store.clone(),
        store_sub: Arc::new(()),
    };

    let mut group = c.benchmark_group("map_handle_shape");

    group.bench_function("clone/wrapped", |b| b.iter(|| black_box(wrapped.clone())));
    group.bench_function("clone/flat", |b| b.iter(|| black_box(flat.clone())));

    group.bench_function("read a field/wrapped", |b| {
        b.iter(|| black_box(wrapped.inner.instance_id))
    });
    group.bench_function("read a field/flat", |b| {
        b.iter(|| black_box(flat.instance_id))
    });

    group.bench_function("read the cache/wrapped", |b| {
        b.iter(|| black_box(wrapped.inner.core.cache.len()))
    });
    group.bench_function("read the cache/flat", |b| {
        b.iter(|| black_box(flat.core.cache.len()))
    });

    black_box((
        &flat.path,
        &flat.store,
        &flat.store_sub,
        &wrapped.inner.path,
    ));
    group.finish();
}

/// The same question with the handle cold, which is the only version that
/// answers it: `HANDLES` distinct handles over a working set past any L3.
///
/// `read a field` walks a permutation, so the misses overlap; `chase a field`
/// takes the next index out of the field just read, so none do. All handles
/// share one core - building that many real ones is three `DashMap`s each -
/// which handicaps the wrapper, not the flat shape.
fn bench_handle_shape_cold(c: &mut Criterion) {
    const HANDLES: usize = 200_000;

    let (_tmp, store) = store("map-handle-shape-cold");
    let path = StorePath::from_segments(["bench", "handle"]);
    let core = ReactiveMapCore::<String, u64>::new();

    let mut wrapped = Vec::with_capacity(HANDLES);
    let mut flat = Vec::with_capacity(HANDLES);
    for _ in 0..HANDLES {
        let id = Uuid::new_v4();
        wrapped.push(Wrapped {
            inner: Arc::new(Parts {
                core: core.clone(),
                path: path.clone(),
                instance_id: id,
                _store: store.clone(),
                _store_sub: Arc::new(()),
            }),
        });
        flat.push(Flat {
            core: core.clone(),
            path: path.clone(),
            instance_id: id,
            store: store.clone(),
            store_sub: Arc::new(()),
        });
    }

    let order = permutation(HANDLES);
    let chain = cycle(HANDLES);
    for (handle, &next) in wrapped.iter_mut().zip(&chain) {
        Arc::get_mut(&mut handle.inner).unwrap().instance_id = Uuid::from_u128(next as u128);
    }
    for (handle, &next) in flat.iter_mut().zip(&chain) {
        handle.instance_id = Uuid::from_u128(next as u128);
    }

    eprintln!(
        "handle sizes: Wrapped {} B, Flat {} B, Parts {} B; \
         working set: wrapped {} KiB of pointers + {} KiB of parts, flat {} KiB",
        size_of::<Wrapped>(),
        size_of::<Flat>(),
        size_of::<Parts>(),
        HANDLES * size_of::<Wrapped>() / 1024,
        HANDLES * size_of::<Parts>() / 1024,
        HANDLES * size_of::<Flat>() / 1024,
    );

    let mut group = c.benchmark_group("map_handle_shape_cold");
    group.throughput(Throughput::Elements(HANDLES as u64));

    group.bench_function("read a field/wrapped", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for &i in &order {
                acc ^= wrapped[i].inner.instance_id.as_u128();
            }
            acc
        })
    });
    group.bench_function("read a field/flat", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for &i in &order {
                acc ^= flat[i].instance_id.as_u128();
            }
            acc
        })
    });

    group.bench_function("chase a field/wrapped", |b| {
        b.iter(|| {
            let mut i = 0usize;
            for _ in 0..HANDLES {
                i = wrapped[i].inner.instance_id.as_u128() as usize;
            }
            i
        })
    });
    group.bench_function("chase a field/flat", |b| {
        b.iter(|| {
            let mut i = 0usize;
            for _ in 0..HANDLES {
                i = flat[i].instance_id.as_u128() as usize;
            }
            i
        })
    });

    group.bench_function("clone/wrapped", |b| {
        b.iter(|| {
            for &i in &order {
                black_box(wrapped[i].clone());
            }
        })
    });
    group.bench_function("clone/flat", |b| {
        b.iter(|| {
            for &i in &order {
                black_box(flat[i].clone());
            }
        })
    });

    group.finish();
}

/// Which lookup is faster, hashed or ordered, for both key types and at the
/// sizes this library is built for.
fn bench_lookup_structure(c: &mut Criterion) {
    use std::collections::{BTreeMap, HashMap};

    const PROBES: usize = 4096;
    const FILLS: [usize; 3] = [10, 1_000, 100_000];

    let mut group = c.benchmark_group("lookup_structure");
    group.throughput(Throughput::Elements(PROBES as u64));

    for fill in FILLS {
        let probes: Vec<usize> = permutation(PROBES.max(fill))
            .into_iter()
            .map(|i| i % fill)
            .take(PROBES)
            .collect();

        let hashed: HashMap<u64, u64> = (0..fill as u64).map(|i| (i, i)).collect();
        let ordered: BTreeMap<u64, u64> = (0..fill as u64).map(|i| (i, i)).collect();

        group.bench_with_input(BenchmarkId::new("u64/hash", fill), &fill, |b, _| {
            b.iter(|| {
                let mut acc = 0u64;
                for &i in &probes {
                    acc ^= hashed[&(i as u64)];
                }
                acc
            })
        });
        group.bench_with_input(BenchmarkId::new("u64/btree", fill), &fill, |b, _| {
            b.iter(|| {
                let mut acc = 0u64;
                for &i in &probes {
                    acc ^= ordered[&(i as u64)];
                }
                acc
            })
        });

        let keys: Vec<String> = (0..fill).map(|i| format!("item-{i:07}")).collect();
        let hashed: HashMap<String, u64> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), i as u64))
            .collect();
        let ordered: BTreeMap<String, u64> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), i as u64))
            .collect();

        group.bench_with_input(BenchmarkId::new("string/hash", fill), &fill, |b, _| {
            b.iter(|| {
                let mut acc = 0u64;
                for &i in &probes {
                    acc ^= hashed[keys[i].as_str()];
                }
                acc
            })
        });
        group.bench_with_input(BenchmarkId::new("string/btree", fill), &fill, |b, _| {
            b.iter(|| {
                let mut acc = 0u64;
                for &i in &probes {
                    acc ^= ordered[keys[i].as_str()];
                }
                acc
            })
        });
    }

    group.finish();
}

/// A fixed shuffle of `0..len`, so the walk order is the same on every run and
/// on both shapes, and no prefetcher can follow it.
fn permutation(len: usize) -> Vec<usize> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut order: Vec<usize> = (0..len).collect();
    for i in (1..len).rev() {
        order.swap(i, (next() % (i as u64 + 1)) as usize);
    }
    order
}

/// A permutation of `0..len` that is one cycle of length `len`, so following
/// `i = cycle[i]` visits every element before it repeats.
///
/// Sattolo's algorithm - Fisher-Yates with the swap partner drawn strictly
/// below `i`, which is exactly the constraint that leaves a single cycle.
fn cycle(len: usize) -> Vec<usize> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut order: Vec<usize> = (0..len).collect();
    for i in (1..len).rev() {
        order.swap(i, (next() % (i as u64)) as usize);
    }
    order
}

/// The store's own scan, with the whole map still in the write buffer.
///
/// `ReactiveMap`'s `len`, `keys` and `entries` answer from the projection and
/// never reach this, so the cost of folding the buffer over the committed rows
/// is invisible from up there - and it is the cost anything addressing the
/// store by path pays.
fn bench_store_scan(c: &mut Criterion) {
    use amethystate::store::StoreBackend;

    let mut group = c.benchmark_group("store_scan_buffered");
    group.sample_size(10);

    for n in OPEN_SIZES {
        let (_tmp, store, _map) = populated("store-scan", n);
        let prefix = amethystate_core::path::StorePath::segment("bench");
        group.throughput(Throughput::Elements(n as u64));

        group.bench_with_input(BenchmarkId::new("scan_keys", n), &n, |b, _| {
            b.iter(|| black_box(StoreBackend::scan_keys(&store, &prefix).unwrap().len()))
        });
        group.bench_with_input(BenchmarkId::new("scan_prefix", n), &n, |b, _| {
            b.iter(|| black_box(StoreBackend::scan_prefix(&store, &prefix).unwrap().len()))
        });

        let (_tmp2, store, _map) = committed("store-scan-committed", n);
        group.bench_with_input(BenchmarkId::new("scan_keys_committed", n), &n, |b, _| {
            b.iter(|| black_box(StoreBackend::scan_keys(&store, &prefix).unwrap().len()))
        });
        group.bench_with_input(BenchmarkId::new("scan_prefix_committed", n), &n, |b, _| {
            b.iter(|| black_box(StoreBackend::scan_prefix(&store, &prefix).unwrap().len()))
        });
    }

    // The state between flushes, where the two lists interleave and the merge
    // has something to do. A handful of rewrites is what an application has
    // buffered a moment after somebody edited something; half is what a burst
    // leaves.
    for n in [1_000usize, 10_000, 100_000] {
        let prefix = amethystate_core::path::StorePath::segment("bench");
        group.throughput(Throughput::Elements(n as u64));

        for pending in [32usize, n / 2] {
            let (_tmp, store, _map) =
                half_flushed(&format!("store-scan-mixed-{n}-{pending}"), n, pending);

            group.bench_with_input(
                BenchmarkId::new(format!("scan_keys_pending{pending}"), n),
                &n,
                |b, _| b.iter(|| black_box(StoreBackend::scan_keys(&store, &prefix).unwrap().len())),
            );
            group.bench_with_input(
                BenchmarkId::new(format!("scan_prefix_pending{pending}"), n),
                &n,
                |b, _| {
                    b.iter(|| black_box(StoreBackend::scan_prefix(&store, &prefix).unwrap().len()))
                },
            );
        }
    }

    group.finish();
}

/// Opening a map over entries that are already there, which is the scan plus
/// everything done with its answer.
///
/// The end of the chain, and the only place that says whether a cost was
/// removed or moved: `load_map` used to parse every key the scan handed it, so
/// a scan that hands back paths can only be judged from here.
fn bench_map_open(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_open_over_existing");
    group.sample_size(10);

    for n in OPEN_SIZES {
        let (_tmp, store, _map) = populated("map-open", n);
        group.throughput(Throughput::Elements(n as u64));

        group.bench_with_input(BenchmarkId::new("open", n), &n, |b, _| {
            b.iter(|| black_box(store.kv().map::<String, u64>("bench").unwrap()))
        });

        let (_tmp2, store, _map) = committed("map-open-committed", n);
        group.bench_with_input(BenchmarkId::new("open_committed", n), &n, |b, _| {
            b.iter(|| black_box(store.kv().map::<String, u64>("bench").unwrap()))
        });
    }

    group.finish();
}

fn bench_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_read");

    for n in SIZES {
        let (_tmp, _store, map) = populated("map-read", n);
        let hit = key(n / 2);
        let miss = "absent".to_string();

        group.bench_with_input(BenchmarkId::new("get_hit", n), &n, |b, _| {
            b.iter(|| black_box(map.get(&hit).unwrap()))
        });
        group.bench_with_input(BenchmarkId::new("get_miss", n), &n, |b, _| {
            b.iter(|| black_box(map.get(&miss).unwrap()))
        });
        group.bench_with_input(BenchmarkId::new("contains_key_hit", n), &n, |b, _| {
            b.iter(|| black_box(map.contains_key(&hit)))
        });
    }

    group.finish();
}

fn bench_subscribers(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_write_vs_subscribers");

    for subs in [0usize, 1, 16, 64, 256, 1024, 4096] {
        let (_tmp, _store, map) = populated("map-subs", 100);
        let handles: Vec<_> = (0..subs)
            .map(|_| {
                map.subscribe_any(|change| {
                    black_box(change);
                })
            })
            .collect();

        group.bench_with_input(BenchmarkId::new("subscribe_any", subs), &subs, |b, _| {
            b.iter(|| black_box(map.update(&key(0), &7).unwrap()))
        });

        drop(handles);
    }

    let (_tmp, _store, map) = populated("map-subkey", 100);
    let _sub = map.subscribe_key(key(0), |change| {
        black_box(change);
    });
    group.bench_function("subscribe_key_hit", |b| {
        b.iter(|| black_box(map.update(&key(0), &7).unwrap()))
    });

    group.finish();
}

fn bench_durability(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_write_durability");
    group.sample_size(30);

    let (_tmp, _store, map) = populated("map-buffered", 100);
    group.bench_function("buffered_insert", |b| {
        b.iter(|| black_box(map.insert(key(0), &7).unwrap()))
    });

    let (_tmp, _store, map) = populated("map-durable", 100);
    group.bench_function("durable_insert_with_commit", |b| {
        b.iter(|| black_box(map.durable().insert(key(0), &7).unwrap()))
    });

    group.finish();
}

/// What inside an open could be done on more than one core, and what it would
/// buy.
///
/// Opening is a scan and then a decode, and the scan is most of it: at a
/// million entries `scan_keys` is 3.7 s of a 6.3 s open before a single value
/// is looked at. So "decode in parallel" aims at the smaller half, and the
/// question worth measuring is what the larger half is made of. Two of the
/// three pieces are ordinary CPU work over independent items and would divide
/// across cores; the third is a walk down a B-tree and would not.
///
/// These measure the pieces on their own, away from the store, because the
/// point is the ceiling rather than the integration: parsing every key and
/// decoding every value are what an open would have to do however it is
/// arranged, and subtracting them from the scan leaves the walk.
fn bench_open_parallelism(c: &mut Criterion) {
    use amethystate_core::path::StorePath;
    use rayon::prelude::*;

    let mut group = c.benchmark_group("open_parallelism");
    group.sample_size(10);

    // Close enough together to find where splitting the work starts paying
    // for itself: below some size the handing out and collecting costs more
    // than the work does, and a decade between samples cannot say where.
    for n in [
        100usize, 300, 1_000, 3_000, 10_000, 30_000, 100_000, 1_000_000,
    ] {
        let joined: Vec<String> = (0..n).map(|i| format!("bench.{}", key(i))).collect();
        let encoded: Vec<Vec<u8>> = (0..n)
            .map(|i| rmp_serde::to_vec(&(i as u64)).unwrap())
            .collect();
        let rows: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                rmp_serde::to_vec(&Row {
                    title: format!("row number {i}"),
                    host: "127.0.0.1".to_string(),
                    port: (i % 65535) as u16,
                    done: i.is_multiple_of(3),
                    updated_at: i as i64,
                })
                .unwrap()
            })
            .collect();

        group.throughput(Throughput::Elements(n as u64));

        // Folded rather than counted: a `map` whose results are thrown away is
        // a computation the compiler may drop, and a benchmark that measures
        // nothing reports something.
        //
        // Folded on `as_str`, not on `len`: a path splits its levels lazily,
        // `len` is one of the things that asks for the split, and a scan never
        // does - so measuring it would price work the scan does not pay.
        group.bench_with_input(BenchmarkId::new("parse_keys_seq", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    joined
                        .iter()
                        .map(|j| StorePath::parse_joined(j).unwrap().as_str().len())
                        .sum::<usize>(),
                )
            })
        });
        group.bench_with_input(BenchmarkId::new("parse_keys_par", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    joined
                        .par_iter()
                        .map(|j| StorePath::parse_joined(j).unwrap().as_str().len())
                        .sum::<usize>(),
                )
            })
        });

        group.bench_with_input(BenchmarkId::new("decode_rows_seq", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    rows.iter()
                        .map(|v| rmp_serde::from_slice::<Row>(v).unwrap())
                        .fold(0i64, |acc, r| acc ^ r.updated_at),
                )
            })
        });
        group.bench_with_input(BenchmarkId::new("decode_rows_par", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    rows.par_iter()
                        .map(|v| rmp_serde::from_slice::<Row>(v).unwrap())
                        .map(|r| r.updated_at)
                        .reduce(|| 0i64, |a, b| a ^ b),
                )
            })
        });

        group.bench_with_input(BenchmarkId::new("decode_values_seq", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    encoded
                        .iter()
                        .map(|v| rmp_serde::from_slice::<u64>(v).unwrap())
                        .fold(0u64, |acc, v| acc ^ v),
                )
            })
        });
        group.bench_with_input(BenchmarkId::new("decode_values_par", n), &n, |b, _| {
            b.iter(|| {
                black_box(
                    encoded
                        .par_iter()
                        .map(|v| rmp_serde::from_slice::<u64>(v).unwrap())
                        .reduce(|| 0u64, |a, b| a ^ b),
                )
            })
        });
    }

    group.finish();
}

criterion_group!(
    reactive_map,
    bench_writes,
    bench_len,
    bench_scans,
    bench_windowed_reads,
    bench_handle_shape,
    bench_handle_shape_cold,
    bench_lookup_structure,
    bench_store_scan,
    bench_map_open,
    bench_reads,
    bench_subscribers,
    bench_durability,
    bench_open_parallelism
);
criterion_main!(reactive_map);
