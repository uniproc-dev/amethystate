use amethystate::store::StorePath;
use amethystate::store::backend::text::document::TextDocument;
use amethystate::store::backend::text::json::json_doc::JsonDocument;
use amethystate::store::backend::text::json::json_tree::JsonTree;
use amethystate::store::backend::text::store::diff_documents;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

const SIZES: [usize; 4] = [100, 1_000, 10_000, 100_000];

/// A declaration, so that one of the shapes below is a tree rather than a
/// plane.
///
/// Without it every key a bench writes lands in the plane - `Declared` reads
/// what the binary declares, and a bench that declares nothing has no tree at
/// all. Which makes the plane the easy shape to measure and the wrong one to
/// draw conclusions from: an application that declares its structs, which is
/// what the macro is for, nests everything, and the plane is what is left for
/// what nobody declared. The map is the wide level such an application has.
#[amethystate::amethystate(prefix = "panels")]
pub struct Panels {
    #[amestate(default = 0)]
    pub width: u32,

    #[amestate(default = 0)]
    pub height: u32,

    #[amestate(default = {})]
    pub items: amethystate::ReactiveMap<String, u64>,
}

/// How much of the document the other reading disagrees about.
#[derive(Clone, Copy)]
enum Changed {
    /// The file came back exactly as it was left, which is what a watcher tick
    /// finds nearly every time.
    Nothing,

    /// A person edited a value or a few, which is what an outside edit is.
    Keys(usize),

    /// A share of the whole, for the shapes an import or a merge makes.
    Share(usize),

    /// Not one key in common: everything was replaced.
    Everything,
}

impl Changed {
    fn label(self) -> String {
        match self {
            Changed::Nothing => "nothing changed".to_string(),
            Changed::Keys(1) => "1 key".to_string(),
            Changed::Keys(n) => format!("{n} keys"),
            Changed::Share(percent) => format!("{percent}%"),
            Changed::Everything => "nothing in common".to_string(),
        }
    }

    /// How many of `keys` hold something else.
    fn among(self, keys: usize) -> usize {
        match self {
            Changed::Nothing => 0,
            Changed::Keys(n) => n.min(keys),
            Changed::Share(percent) => keys * percent / 100,
            Changed::Everything => keys,
        }
    }

    /// Whether the keys themselves are different ones, rather than the same
    /// keys holding something else.
    fn renames(self) -> bool {
        matches!(self, Changed::Everything)
    }
}

const PROFILES: [Changed; 6] = [
    Changed::Nothing,
    Changed::Keys(1),
    Changed::Keys(10),
    Changed::Share(25),
    Changed::Share(50),
    Changed::Everything,
];

fn flat(keys: usize, changed: Changed) -> String {
    let differing = changed.among(keys);
    let renames = changed.renames();

    let held: serde_json::Map<String, serde_json::Value> = (0..keys)
        .map(|at| {
            let name = match renames && at < differing {
                true => format!("widget{at}.width"),
                false => format!("plugin{at}.width"),
            };
            let value = match !renames && at < differing {
                true => 9_999 + at,
                false => at,
            };
            (name, serde_json::Value::Number(value.into()))
        })
        .collect();

    serde_json::to_string(&serde_json::Value::Object(held)).expect("the source renders")
}

fn nested(keys: usize, changed: Changed) -> String {
    let differing = changed.among(keys);
    let renames = changed.renames();

    let held: serde_json::Map<String, serde_json::Value> = (0..keys)
        .map(|at| {
            let name = match renames && at < differing {
                true => format!("widget{at}"),
                false => format!("plugin{at}"),
            };
            let width = match !renames && at < differing {
                true => 9_999 + at,
                false => at,
            };
            (
                name,
                serde_json::json!({
                    "width": width,
                    "height": at * 2,
                    "title": format!("the {at}th panel, named at some length"),
                    "flags": ["open", "pinned", "resizable"],
                }),
            )
        })
        .collect();

    serde_json::to_string(&serde_json::Value::Object(held)).expect("the source renders")
}

/// The shape an application that declares its structs actually stores: a
/// declared prefix holding fields and a map, so the root name is a tree and the
/// walk descends into it.
fn declared(keys: usize, changed: Changed) -> String {
    let differing = changed.among(keys);
    let renames = changed.renames();

    let items: serde_json::Map<String, serde_json::Value> = (0..keys)
        .map(|at| {
            let name = match renames && at < differing {
                true => format!("widget{at}"),
                false => format!("panel{at}"),
            };
            let value = match !renames && at < differing {
                true => 9_999 + at,
                false => at,
            };
            (name, serde_json::Value::Number(value.into()))
        })
        .collect();

    serde_json::to_string(&serde_json::json!({
        "panels": {
            "width": 1280,
            "height": 720,
            "items": items,
        }
    }))
    .expect("the source renders")
}

fn arms<D: TextDocument>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    engine: &str,
    keys: usize,
    src: &str,
) {
    let doc = D::parse(src).expect("the source parses");
    let rendered = doc.serialize().expect("the document renders");

    group.bench_with_input(
        BenchmarkId::new(format!("clone, {engine}"), keys),
        &doc,
        |b, doc| {
            b.iter(|| black_box(doc.clone()));
        },
    );

    group.bench_with_input(
        BenchmarkId::new(format!("render, {engine}"), keys),
        &doc,
        |b, doc| {
            b.iter(|| black_box(doc.serialize().unwrap()));
        },
    );

    group.bench_with_input(
        BenchmarkId::new(format!("parse, {engine}"), keys),
        &rendered,
        |b, text| {
            b.iter(|| black_box(D::parse(text).unwrap()));
        },
    );

    group.bench_with_input(
        BenchmarkId::new(format!("scan, one pass, {engine}"), keys),
        &doc,
        |b, doc| {
            b.iter(|| black_box(doc.scan(&StorePath::root()).unwrap()));
        },
    );
}

fn diff_arms<D: TextDocument>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    engine: &str,
    keys: usize,
    src: &str,
    build: fn(usize, Changed) -> String,
) {
    let doc = D::parse(src).expect("the source parses");

    for profile in PROFILES {
        let source = build(keys, profile);

        // Parsed per iteration, and deliberately not once: a node that answers
        // `known_same` from a hash works the answer out on the first ask and
        // keeps it, so a pair handed to criterion whole is hashed once and
        // compared for free ever after. That is not what a watcher pays. The
        // reading it holds stays warm across ticks - nothing writes to it - but
        // the one off the disk is new every time, and its hash is part of the
        // tick. `iter_batched` leaves the parse out of the measurement and the
        // hashing in, which is the half this is asking about.
        group.bench_with_input(
            BenchmarkId::new(format!("{}, {engine}", profile.label()), keys),
            &source,
            |b, source| {
                b.iter_batched(
                    || D::parse(source).expect("the other reading parses"),
                    |after| black_box(diff_documents::<D>(&doc, &after, 1).unwrap()),
                    criterion::BatchSize::SmallInput,
                );
            },
        );

        let after = D::parse(&source).expect("the other reading parses");
        let said = diff_documents::<D>(&doc, &after, 1).expect("the diff runs");
        assert_eq!(
            said.is_empty(),
            matches!(profile, Changed::Nothing),
            "the {} profile on {engine} reported {} events, so this row times a walk that \
             finds nothing",
            profile.label(),
            said.len()
        );
    }
}

/// What hashing the whole document at parse would cost, against what it would
/// save: the name comparisons a diff makes to find out which names moved.
///
/// The trade is not obvious in either direction. A deep document wins - one
/// comparison stands for a whole subtree - but the plane is one level of
/// scalars, where hashing walks every value to save a comparison per child.
fn bench_hash(c: &mut Criterion) {
    use amethystate::store::backend::text::tree::subtree_hash;

    let mut group = c.benchmark_group("hashing a document at parse");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    for (shape, build) in [
        ("flat scalars", flat as fn(usize, Changed) -> String),
        ("object values", nested),
        ("a declared tree", declared),
    ] {
        for keys in SIZES {
            let src = build(keys, Changed::Nothing);
            let doc = JsonTree::parse(&src).expect("it parses");
            let root = doc.get(&StorePath::root()).expect("a document has a root");

            group.throughput(Throughput::Elements(keys as u64));
            group.bench_with_input(BenchmarkId::new(shape, keys), root, |b, root| {
                b.iter(|| black_box(subtree_hash(root)));
            });

            // What the watcher pays instead, on a file that was touched and not
            // changed: one pass over the bytes it read anyway, against a parse
            // of the whole file and two renders to compare against.
            group.bench_with_input(
                BenchmarkId::new(format!("{shape}, the file's bytes"), keys),
                &src,
                |b, src| {
                    b.iter(|| black_box(xxhash_rust::xxh3::xxh3_128(src.as_bytes())));
                },
            );
        }
    }

    group.finish();
}

/// Where a wide level's time goes, which is what decides whether chunking it is
/// worth building.
///
/// A diff that descends into a map of a hundred thousand entries asks the level
/// for its children, puts both sides in name order, and walks them together.
/// Chunk hashes kept *in* the level would skip all three - so the question is
/// how much of the walk is the walking and how much is getting ready to walk.
fn bench_wide_level(c: &mut Criterion) {
    use amethystate::store::backend::text::document::Navigable;

    let mut group = c.benchmark_group("a wide level");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    for keys in SIZES {
        let doc = JsonTree::parse(&declared(keys, Changed::Nothing)).expect("it parses");
        let items = doc
            .get(&StorePath::from_segments(["panels", "items"]))
            .expect("the declared map is there");

        group.throughput(Throughput::Elements(keys as u64));

        group.bench_with_input(BenchmarkId::new("children", keys), items, |b, items| {
            b.iter(|| black_box(items.children()));
        });

        group.bench_with_input(
            BenchmarkId::new("children, in name order", keys),
            items,
            |b, items| {
                b.iter(|| {
                    let mut held = items.children();
                    held.sort_by(|(ours, _), (theirs, _)| ours.cmp(theirs));
                    black_box(held)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("children, checked for one order", keys),
            items,
            |b, items| {
                b.iter(|| {
                    let (mine, theirs) = (items.children(), items.children());
                    black_box(
                        mine.len() == theirs.len()
                            && mine
                                .iter()
                                .zip(&theirs)
                                .all(|((ours, _), (theirs, _))| ours == theirs),
                    )
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("one comparison per child", keys),
            items,
            |b, items| {
                let held = items.children();
                b.iter(|| {
                    let mut same = 0usize;
                    for (_, node) in &held {
                        if node.known_same(node) {
                            same += 1;
                        }
                    }
                    black_box(same)
                });
            },
        );
    }

    group.finish();
}

fn bench_document(c: &mut Criterion) {
    for (shape, build) in [
        ("flat scalars", flat as fn(usize, Changed) -> String),
        ("object values", nested),
        ("a declared tree", declared),
    ] {
        let mut group = c.benchmark_group(format!("text document, {shape}"));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(5));
        group.warm_up_time(Duration::from_secs(1));

        for keys in SIZES {
            let src = build(keys, Changed::Nothing);

            group.throughput(Throughput::Elements(keys as u64));

            arms::<JsonDocument>(&mut group, "reference", keys, &src);
            arms::<JsonTree>(&mut group, "owned tree", keys, &src);
        }

        group.finish();

        let mut group = c.benchmark_group(format!("diff, {shape}"));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(5));
        group.warm_up_time(Duration::from_secs(1));

        for keys in SIZES {
            let src = build(keys, Changed::Nothing);

            group.throughput(Throughput::Elements(keys as u64));

            diff_arms::<JsonDocument>(&mut group, "reference", keys, &src, build);
            diff_arms::<JsonTree>(&mut group, "owned tree", keys, &src, build);
        }

        group.finish();
    }
}

criterion_group!(benches, bench_wide_level, bench_hash, bench_document);
criterion_main!(benches);
