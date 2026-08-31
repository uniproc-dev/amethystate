//! Laying the write buffer over the engine's answer, on its own: `allocating`
//! against `in_place`, and `shipped` for whichever the library runs. Both
//! algorithms live here so the choice between them stays checkable.
//!
//! The shapes are the states a store is in - after a flush, a moment after an
//! edit, after a burst, and before the first flush.

use amethystate::store::backend::utils::merge_buffered;
use amethystate_core::path::StorePath;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

type Committed = Vec<(StorePath, Vec<u8>)>;
type Buffered = Vec<(StorePath, Option<Vec<u8>>)>;

/// Two iterators feeding a third list.
fn merge_allocating(committed: Committed, buffered: Buffered) -> Committed {
    let mut out = Vec::with_capacity(committed.len() + buffered.len());
    let mut left = committed.into_iter().peekable();
    let mut right = buffered.into_iter().peekable();

    loop {
        let take_left = match (left.peek(), right.peek()) {
            (Some((a, _)), Some((b, _))) => a <= b,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };

        if take_left {
            let (key, value) = left.next().expect("peeked");
            if right.peek().is_some_and(|(b, _)| *b == key) {
                continue;
            }
            out.push((key, value));
        } else {
            let (key, value) = right.next().expect("peeked");
            if let Some(value) = value {
                out.push((key, value));
            }
        }
    }

    out
}

/// The same walk backwards into `committed`'s own tail, then shifted down.
fn merge_in_place(mut committed: Committed, mut buffered: Buffered) -> Committed {
    fn empty_slot() -> (StorePath, Vec<u8>) {
        (StorePath::root(), Vec::new())
    }

    if buffered.is_empty() {
        return committed;
    }

    let mut c = committed.len();
    let mut b = buffered.len();
    let mut w = c + b;
    committed.resize(w, empty_slot());

    while c > 0 && b > 0 {
        match committed[c - 1].0.cmp(&buffered[b - 1].0) {
            std::cmp::Ordering::Greater => {
                c -= 1;
                w -= 1;
                committed[w] = std::mem::replace(&mut committed[c], empty_slot());
            }
            std::cmp::Ordering::Less => {
                b -= 1;
                if let (key, Some(value)) = buffered.pop().expect("b > 0") {
                    w -= 1;
                    committed[w] = (key, value);
                }
            }
            std::cmp::Ordering::Equal => {
                c -= 1;
                b -= 1;
                committed[c] = empty_slot();
                if let (key, Some(value)) = buffered.pop().expect("b > 0") {
                    w -= 1;
                    committed[w] = (key, value);
                }
            }
        }
    }

    while c > 0 {
        c -= 1;
        w -= 1;
        committed[w] = std::mem::replace(&mut committed[c], empty_slot());
    }

    while b > 0 {
        b -= 1;
        if let (key, Some(value)) = buffered.pop().expect("b > 0") {
            w -= 1;
            committed[w] = (key, value);
        }
    }

    committed.drain(..w);
    committed
}

fn key(i: usize) -> StorePath {
    StorePath::from_segments(["bench", &format!("k{i:07}")])
}

/// A value the size of a small stored struct.
fn value(i: usize) -> Vec<u8> {
    vec![(i % 251) as u8; 48]
}

/// `n` committed entries with `pending` of them written again, spread across
/// the range; `deletes` of those remove their key instead of replacing it.
fn lists(n: usize, pending: usize, deletes: usize) -> (Committed, Buffered) {
    let committed: Committed = (0..n).map(|i| (key(i), value(i))).collect();

    let stride = (n / pending.max(1)).max(1);
    let buffered: Buffered = (0..n)
        .step_by(stride)
        .take(pending)
        .enumerate()
        .map(|(seen, i)| {
            let op = if seen < deletes {
                None
            } else {
                Some(value(i + 1))
            };
            (key(i), op)
        })
        .collect();

    (committed, buffered)
}

/// Everything in the buffer and nothing committed: a store before its first
/// flush.
fn all_buffered(n: usize) -> (Committed, Buffered) {
    (
        Vec::new(),
        (0..n).map(|i| (key(i), Some(value(i)))).collect(),
    )
}

fn bench_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan_merge");

    for n in [1_000usize, 10_000, 100_000] {
        group.throughput(Throughput::Elements(n as u64));

        let shapes: [(&str, (Committed, Buffered)); 5] = [
            ("nothing buffered", (lists(n, 0, 0).0, Vec::new())),
            ("32 buffered", lists(n, 32, 0)),
            ("32 buffered, half of them deletes", lists(n, 32, 16)),
            ("half buffered", lists(n, n / 2, 0)),
            ("all buffered", all_buffered(n)),
        ];

        {
            let answer: Committed = (0..n).map(|i| (key(i), value(i))).collect();
            group.bench_with_input(BenchmarkId::new("free the answer", n), &n, |b, _| {
                b.iter_batched(|| answer.clone(), drop, BatchSize::LargeInput)
            });
        }

        for (shape, (committed, buffered)) in shapes {
            let arms: [(&str, fn(Committed, Buffered) -> Committed, bool); 3] = [
                ("allocating", merge_allocating, false),
                ("in_place", merge_in_place, true),
                ("shipped", merge_buffered, false),
            ];

            for (name, merge, reserved) in arms {
                let setup = || {
                    let mut committed = committed.clone();
                    if reserved {
                        committed.reserve(buffered.len());
                    }
                    (committed, buffered.clone())
                };

                group.bench_with_input(
                    BenchmarkId::new(format!("{name}/{shape}"), n),
                    &n,
                    |b, _| b.iter_batched(setup, |(c, f)| merge(c, f), BatchSize::LargeInput),
                );

                group.bench_with_input(
                    BenchmarkId::new(format!("{name} and free it/{shape}"), n),
                    &n,
                    |b, _| {
                        b.iter_batched(
                            setup,
                            |(c, f)| black_box(merge(c, f).len()),
                            BatchSize::LargeInput,
                        )
                    },
                );
            }
        }
    }

    group.finish();
}

criterion_group!(scan_merge, bench_merge);
criterion_main!(scan_merge);
