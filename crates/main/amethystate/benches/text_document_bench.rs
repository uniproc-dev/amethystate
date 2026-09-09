use amethystate::store::StorePath;
use amethystate::store::backend::text::document::TextDocument;
use amethystate::store::backend::text::json::json_doc::JsonDocument;
use amethystate::store::backend::text::json::json_tree::JsonTree;
use amethystate::store::backend::text::store::diff_documents;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

const SIZES: [usize; 4] = [100, 1_000, 10_000, 100_000];

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
        let after = D::parse(&build(keys, profile)).expect("the other reading parses");

        group.bench_with_input(
            BenchmarkId::new(format!("{}, {engine}", profile.label()), keys),
            &(&doc, &after),
            |b, (old, new)| {
                b.iter(|| black_box(diff_documents::<D>(old, new, 1).unwrap()));
            },
        );
    }
}

fn bench_document(c: &mut Criterion) {
    for (shape, build) in [
        ("flat scalars", flat as fn(usize, Changed) -> String),
        ("object values", nested),
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

criterion_group!(benches, bench_document);
criterion_main!(benches);
