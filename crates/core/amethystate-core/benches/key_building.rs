//! What a key costs to build, and whether the buffer should be inline.
//!
//! `StorePath::key` is on every read and every write, so the allocation it
//! makes is paid once per operation rather than once per scan. A key is short -
//! the levels plus one byte each - so an inline buffer would hold nearly all of
//! them, at the price of a wider `Key` that every caller then moves around.

use amethystate_core::path::{Key, StorePath};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use smallvec::SmallVec;
use std::hint::black_box;

/// The same encoding, into an inline buffer.
fn inline<const N: usize>(path: &StorePath) -> SmallVec<[u8; N]> {
    let mut out: SmallVec<[u8; N]> = SmallVec::new();

    for level in path.segments() {
        for &byte in level.as_str().as_bytes() {
            out.push(byte);
            if byte == 0 {
                out.push(0xFF);
            }
        }
        out.push(0);
    }

    out
}

/// The same encoding, into a buffer the caller keeps.
fn reused(path: &StorePath, out: &mut amethystate_core::path::KeyBytes) {
    out.clear();
    path.write_key(out);
}

fn shapes() -> Vec<(&'static str, StorePath)> {
    vec![
        ("one level", StorePath::from_segments(["width"])),
        ("two levels", StorePath::from_segments(["ui", "theme"])),
        (
            "a declared entry",
            StorePath::from_segments(["panels", "items", "panel12345"]),
        ),
        (
            "six levels",
            StorePath::from_segments(["a", "b", "c", "d", "e", "f"]),
        ),
        (
            "eight levels, named",
            StorePath::from_segments([
                "workspace",
                "windows",
                "main",
                "panels",
                "inspector",
                "sections",
                "layout",
                "width",
            ]),
        ),
        (
            "past any inline buffer",
            StorePath::from_segments([
                "an application that names its levels at some length",
                "and then another one just as long as the first",
                "and a third",
            ]),
        ),
    ]
}

fn bench_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("building a key");

    for (shape, path) in shapes() {
        group.bench_with_input(
            BenchmarkId::new("Key, as it is", shape),
            &path,
            |b, path| {
                b.iter(|| black_box(black_box(path).key()));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("SmallVec, 32 inline", shape),
            &path,
            |b, path| {
                b.iter(|| black_box(inline::<32>(black_box(path))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("SmallVec, 48 inline", shape),
            &path,
            |b, path| {
                b.iter(|| black_box(inline::<48>(black_box(path))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("SmallVec, 64 inline", shape),
            &path,
            |b, path| {
                b.iter(|| black_box(inline::<64>(black_box(path))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("SmallVec, 96 inline", shape),
            &path,
            |b, path| {
                b.iter(|| black_box(inline::<96>(black_box(path))));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("a buffer kept between keys", shape),
            &path,
            |b, path| {
                let mut out = amethystate_core::path::KeyBytes::new();
                b.iter(|| {
                    reused(path, &mut out);
                    black_box(out.len())
                });
            },
        );
    }

    group.finish();
}

/// Reading a key back, which is what every key of every scan costs.
fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("reading a key back");

    for (shape, path) in shapes() {
        let held = path.key();
        group.bench_with_input(BenchmarkId::new("path", shape), &held, |b, held| {
            b.iter(|| black_box(held.path().unwrap()));
        });
    }

    group.finish();
}

/// A flush writes one key per pending change, so the buffer a scan could keep
/// is worth measuring against the allocation it would save.
fn bench_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("a thousand keys");

    let paths: Vec<StorePath> = (0..1_000)
        .map(|at| StorePath::from_segments(["panels", "items", &format!("panel{at}")]))
        .collect();

    group.bench_function("Key, boxed", |b| {
        b.iter(|| {
            let mut held: Vec<Key> = Vec::with_capacity(paths.len());
            held.extend(paths.iter().map(|path| path.key()));
            black_box(held.len())
        });
    });

    group.bench_function("SmallVec, 32 inline", |b| {
        b.iter(|| {
            let mut held: Vec<SmallVec<[u8; 32]>> = Vec::with_capacity(paths.len());
            held.extend(paths.iter().map(inline::<32>));
            black_box(held.len())
        });
    });

    group.bench_function("a buffer kept between keys", |b| {
        let mut out = amethystate_core::path::KeyBytes::new();
        b.iter(|| {
            let mut total = 0usize;
            for path in &paths {
                reused(path, &mut out);
                total += out.len();
            }
            black_box(total)
        });
    });

    group.finish();
}

/// What the path itself costs to build, which is the allocation before the key
/// is even reached.
///
/// `Levels` holds `Box<[SmolStr]>` behind an `Arc`, so a path built from levels
/// is a `Vec` grown, then shrunk into a box, then put in an `Arc` - two
/// allocations where the names are nearly always few and inline.
fn bench_path(c: &mut Criterion) {
    use amethystate_core::path::SmolStr;
    use std::sync::{Arc, OnceLock};

    struct Boxed {
        #[allow(dead_code)]
        names: Box<[SmolStr]>,
        #[allow(dead_code)]
        joined: OnceLock<Arc<str>>,
    }

    struct Inline<const N: usize> {
        #[allow(dead_code)]
        names: SmallVec<[SmolStr; N]>,
        #[allow(dead_code)]
        joined: OnceLock<Arc<str>>,
    }

    let mut group = c.benchmark_group("building a path");

    for (shape, levels) in [
        ("one level", vec!["width"]),
        ("two levels", vec!["ui", "theme"]),
        ("a declared entry", vec!["panels", "items", "panel12345"]),
        ("six levels", vec!["a", "b", "c", "d", "e", "f"]),
    ] {
        group.bench_with_input(
            BenchmarkId::new("StorePath, as it is", shape),
            &levels,
            |b, levels| {
                b.iter(|| black_box(StorePath::from_segments(levels.iter().copied())));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("Vec then boxed, in an Arc", shape),
            &levels,
            |b, levels| {
                b.iter(|| {
                    let names: Vec<SmolStr> = levels.iter().map(SmolStr::new).collect();
                    black_box(Arc::new(Boxed {
                        names: names.into_boxed_slice(),
                        joined: OnceLock::new(),
                    }))
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("SmallVec, 4 inline, in an Arc", shape),
            &levels,
            |b, levels| {
                b.iter(|| {
                    let names: SmallVec<[SmolStr; 4]> = levels.iter().map(SmolStr::new).collect();
                    black_box(Arc::new(Inline::<4> {
                        names,
                        joined: OnceLock::new(),
                    }))
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("SmallVec, 8 inline, in an Arc", shape),
            &levels,
            |b, levels| {
                b.iter(|| {
                    let names: SmallVec<[SmolStr; 8]> = levels.iter().map(SmolStr::new).collect();
                    black_box(Arc::new(Inline::<8> {
                        names,
                        joined: OnceLock::new(),
                    }))
                });
            },
        );
    }

    group.finish();
}

/// What a clone costs, which is what a registry would take away.
///
/// A path is copied far more often than it is built - a scan hands one to the
/// buffer, the buffer to an event, the event to a subscriber - and every copy
/// is an atomic increment on a count that exists only so somebody frees the
/// levels. Against a path that is never freed, a copy is a pointer.
fn bench_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("copying a path");

    let built = StorePath::from_segments(["panels", "items", "panel12345"]);
    const WRITTEN: StorePath =
        StorePath::from_static(&["panels", "items", "cpu"], "panels.items.cpu");

    group.bench_function("built from levels, an Arc", |b| {
        b.iter(|| black_box(black_box(&built).clone()));
    });

    group.bench_function("written in the source, a pointer", |b| {
        b.iter(|| black_box(black_box(&WRITTEN).clone()));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_key,
    bench_decode,
    bench_many,
    bench_path,
    bench_clone
);
criterion_main!(benches);
