//! A scan, on its own, for a profiler to look at.
//!
//! `reactive_map_bench` says what a scan costs and cannot say where the time
//! goes: at a million entries an open is seconds, the disk accounts for
//! milliseconds of it, and parsing and decoding measured on their own account
//! for a fraction. The rest is somewhere in building the answer, and which
//! part is a question for a sampling profiler rather than for arithmetic over
//! totals.
//!
//! Run it under one:
//!
//! ```text
//! cargo flamegraph --example profile_scan --release
//! samply record -- target/release/examples/profile_scan
//! ```
//!
//! The store is committed before the scans, so the write buffer is empty and
//! the work is the engine's, and the population is a separate phase whose
//! frames sit under `insert` rather than under `scan_keys`.

use amethystate::store::StoreBackend;
use amethystate::{Store, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::time::{Duration, Instant};

const SCANS: usize = 3;

/// A stored value with fields in it, which is what a declared struct is.
///
/// Measuring on a `u64` measures the store and not the codec: an integer
/// decodes in about eleven nanoseconds, where this takes two hundred, and the
/// difference is most of what opening a large collection costs.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default, PartialEq)]
struct Row {
    title: String,
    host: String,
    port: u16,
    done: bool,
    updated_at: i64,
}

fn row(i: usize) -> Row {
    Row {
        title: format!("row number {i}"),
        host: "127.0.0.1".to_string(),
        port: (i % 65535) as u16,
        done: i.is_multiple_of(3),
        updated_at: i as i64,
    }
}

/// Counts every allocation and attributes it to where it was made, which a
/// sampling profiler cannot: it says a third of the run is in the allocator
/// and not which lines put it there.
///
/// Recording a backtrace per allocation is slow enough that the sizes here are
/// meant to be turned down - the shape of what one entry costs does not change
/// with how many there are.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let entries: usize = std::env::var("ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);

    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    let parallel = std::env::var("PARALLEL").is_ok_and(|v| v != "0");
    println!("parallel_reads: {parallel}");

    let path = TempPath::new("profile-scan");
    let store: Store = StoreBuilder::new(path.path())
        .disk(|d| d.debounce(Duration::from_secs(600)))
        .parallel_reads(parallel)
        .build()
        .unwrap();

    let map = store.kv().map::<String, Row>("bench").unwrap();

    let t = Instant::now();
    for i in 0..entries {
        map.insert(format!("k{i:07}"), &row(i)).unwrap();
    }
    println!("populate {entries}: {:?}", t.elapsed());

    let t = Instant::now();
    StoreBackend::save_now(&store).unwrap();
    println!("commit: {:?}", t.elapsed());

    let prefix = StorePath::segment("bench");

    for _ in 0..SCANS {
        let t = Instant::now();
        let n = StoreBackend::scan_keys(&store, &prefix).unwrap().len();
        println!("scan_keys {n}: {:?}", t.elapsed());
    }

    for _ in 0..SCANS {
        let t = Instant::now();
        let n = StoreBackend::scan_prefix(&store, &prefix).unwrap().len();
        println!("scan_prefix {n}: {:?}", t.elapsed());
    }

    for _ in 0..SCANS {
        let t = Instant::now();
        let opened = store.kv().map::<String, Row>("bench").unwrap();
        println!("open {}: {:?}", opened.len(), t.elapsed());
    }
}
