//! What each candidate projection holds resident, and what an old snapshot
//! costs while it stays alive.
//!
//! The counting allocator lives here rather than in `map_snapshot_bench` on
//! purpose: it adds a relaxed atomic to every allocation and free, which is
//! exactly the traffic the persistent maps generate, so leaving it in the timing
//! binary would tilt the numbers it is meant to compare. This target is a plain
//! `main` that prints a table; `cargo bench --bench map_snapshot_memory` runs it.
//!
//! Three figures per shape. The first is what the whole map occupies when
//! nothing else refers to it. The other two are what one write and what a
//! hundred writes retain while a snapshot taken before them is still held -
//! the cost the RFC names as "more memory held while an old snapshot is alive".
//! Both the snapshot and each new version are held through an `Arc`, which is
//! what `ArcSwap` would hand out, so the copying shape is charged one whole
//! extra map and neither persistent shape is charged for what it shares.

use im::OrdMap;
use rpds::RedBlackTreeMapSync;
use smol_str::SmolStr;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The system allocator with a running total of what it has handed out.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

type Entry = (String, u64);

const SIZES: [usize; 3] = [1_000, 10_000, 100_000];
const WRITES: usize = 100;

fn name(i: usize) -> SmolStr {
    SmolStr::new(format!("item-{i:07}"))
}

fn entries(n: usize) -> Vec<(SmolStr, Entry)> {
    (0..n)
        .map(|i| {
            let key = name(i * 2);
            (key.clone(), (key.to_string(), i as u64))
        })
        .collect()
}

fn probes(n: usize) -> Vec<SmolStr> {
    (0..WRITES).map(|j| name(j * n / WRITES * 2)).collect()
}

fn written() -> Entry {
    ("payload-value".to_string(), 42)
}

/// The three figures, in bytes.
struct Cost {
    resident: usize,
    after_one: usize,
    after_many: usize,
}

fn measure<M, B, W>(source: &[(SmolStr, Entry)], keys: &[SmolStr], build: B, write: W) -> Cost
where
    B: Fn(&[(SmolStr, Entry)]) -> M,
    W: Fn(&M, &SmolStr, &Entry) -> M,
{
    let before = live();
    let map = Arc::new(build(source));
    let resident = live().saturating_sub(before);

    let held = Arc::clone(&map);
    let value = written();
    let held_at = live();

    let mut current = map;
    let mut after_one = 0;

    for (done, key) in keys.iter().enumerate() {
        current = Arc::new(write(&current, key, &value));
        if done == 0 {
            after_one = live().saturating_sub(held_at);
        }
    }

    let after_many = live().saturating_sub(held_at);

    black_box(&held);
    black_box(&current);
    drop(current);
    drop(held);

    Cost {
        resident,
        after_one,
        after_many,
    }
}

fn btreemap(source: &[(SmolStr, Entry)]) -> BTreeMap<SmolStr, Entry> {
    source.iter().cloned().collect()
}

fn im_map(source: &[(SmolStr, Entry)]) -> OrdMap<SmolStr, Entry> {
    let mut map = OrdMap::new();
    for (key, entry) in source {
        map.insert(key.clone(), entry.clone());
    }
    map
}

fn rpds_map(source: &[(SmolStr, Entry)]) -> RedBlackTreeMapSync<SmolStr, Entry> {
    let mut map = RedBlackTreeMapSync::new_sync();
    for (key, entry) in source {
        map = map.insert(key.clone(), entry.clone());
    }
    map
}

fn row(shape: &str, size: usize, cost: Cost) {
    let per_entry = cost.resident as f64 / size as f64;

    println!(
        "{shape:<16} {size:>8}  {:>12}  {per_entry:>10.1}  {:>14}  {:>16}",
        cost.resident, cost.after_one, cost.after_many
    );
}

fn main() {
    println!(
        "{:<16} {:>8}  {:>12}  {:>10}  {:>14}  {:>16}",
        "shape", "entries", "resident B", "B/entry", "held, 1 write", "held, 100 writes"
    );

    for size in SIZES {
        let source = entries(size);
        let keys = probes(size);

        row(
            "btreemap",
            size,
            measure(
                &source,
                &keys,
                btreemap,
                |map: &BTreeMap<SmolStr, Entry>, key, value| {
                    let mut next = map.clone();
                    next.insert(key.clone(), value.clone());
                    next
                },
            ),
        );

        row(
            "im::OrdMap",
            size,
            measure(
                &source,
                &keys,
                im_map,
                |map: &OrdMap<SmolStr, Entry>, key, value| {
                    let mut next = map.clone();
                    next.insert(key.clone(), value.clone());
                    next
                },
            ),
        );

        row(
            "rpds::RBTreeMap",
            size,
            measure(
                &source,
                &keys,
                rpds_map,
                |map: &RedBlackTreeMapSync<SmolStr, Entry>, key, value| {
                    map.insert(key.clone(), value.clone())
                },
            ),
        );

        println!();
    }
}
