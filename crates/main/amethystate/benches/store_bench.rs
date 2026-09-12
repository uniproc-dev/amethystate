use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use rayon::iter::IntoParallelIterator;
use rayon::iter::ParallelIterator;

use amethystate::store::StorePath;
use amethystate::{Store, StoreBuilder};
use serde::Serialize;
use std::hint::black_box;
use std::time::Duration;

#[derive(Serialize)]
struct BenchData {
    id: u64,
    name: String,
    payload: Vec<u8>,
}

#[cfg(feature = "redb")]
fn setup_store() -> Store {
    let path = std::env::temp_dir().join(format!("bench_{}.redb", rand::random::<u32>()));
    if path.exists() {
        std::fs::remove_file(&path).ok();
    }

    StoreBuilder::new(&path)
        .disk(|d| d.debounce(Duration::from_secs(100)))
        .build()
        .unwrap()
}

#[cfg(feature = "redb")]
#[allow(clippy::unit_arg)]
fn bench_set_load(c: &mut Criterion) {
    let store = setup_store();
    let threads_counts = [1, 4];

    let mut group = c.benchmark_group("RedbStore_Set_Contention");

    for threads in threads_counts {
        let data = BenchData {
            id: 1,
            name: "benchmark_key_value_long_string".to_string(),
            payload: vec![0u8; 128],
        };

        group.bench_with_input(BenchmarkId::new("hot_key", threads), &threads, |b, &t| {
            b.iter(|| {
                (0..t).collect::<Vec<_>>().into_par_iter().for_each(|_| {
                    black_box(store.set(["global", "hot_key"], &data).unwrap());
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("wide_keys", threads), &threads, |b, &t| {
            b.iter(|| {
                (0..t).collect::<Vec<_>>().into_par_iter().for_each(|i| {
                    let key = StorePath::from_segments(["path", &format!("node_{}", i)]);
                    black_box(store.set(&key, &data).unwrap());
                });
            });
        });
    }
    group.finish();
}

#[cfg(not(feature = "redb"))]
fn bench_set_load(_: &mut Criterion) {}

criterion_group!(benches, bench_set_load);
criterion_main!(benches);
