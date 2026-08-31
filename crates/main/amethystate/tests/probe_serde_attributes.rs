//! What a serde attribute does to a store that addresses values by path.
//!
//! The library's model is that a Rust type says where things live: a field name
//! becomes a path level, the schema snapshot records that name, and a read
//! addresses the same name back. A serde attribute changes what a type
//! *serialises as* without changing what it *is*, so every probe here aims at
//! the gap between the two.
//!
//! Every probe does the same thing: build a store on a named engine, write,
//! flush, drop the store, reopen from the file, read back, compare. Nothing is
//! asserted about what *should* happen - the tests print a table and only fail
//! when a probe cannot be run at all. The point is the table.
//!
//! Five verdicts are separated:
//!
//! - **opaque failure** - the file or the store will not open afterwards.
//! - **silent alteration** - the write returned `Ok` and the read gives back
//!   something else, or nothing, or an error.
//! - **residue** - a path nobody wrote is readable, or one that was written is
//!   not reachable.
//! - **schema disagreement** - the snapshot on disk describes something the
//!   data does not match.
//! - **ok** - it works.
//!
//! Every engine the build enabled runs every probe, because they differ and a
//! defect on one is often invisible on another.

#![allow(dead_code)]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{Store, amethystate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

mod common;

/// One line of the report.
struct Row {
    engine: String,
    probe: String,
    wrote: String,
    write: String,
    read: String,
    verdict: String,
}

impl Row {
    fn panicked(engine: &str, probe: &str, what: String) -> Self {
        Row {
            engine: engine.to_string(),
            probe: probe.to_string(),
            wrote: "-".to_string(),
            write: "-".to_string(),
            read: "-".to_string(),
            verdict: format!("PANIC: {}", brief(&what)),
        }
    }
}

/// A cell a markdown table can hold: one line, bounded, no pipes.
fn brief(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| match c {
            '|' => '/',
            '\n' | '\r' | '\t' => ' ',
            '\0' => '␀',
            c if (c as u32) < 0x20 => '␦',
            c => c,
        })
        .collect();

    let mut out: String = flat.chars().take(160).collect();
    if flat.chars().count() > 160 {
        out.push_str(&format!(" …({} chars)", flat.chars().count()));
    }
    out
}

fn table(section: &str, rows: &[Row]) {
    println!("\n## {section}\n");
    println!("| engine | probe | written | write returned | read back | verdict |");
    println!("|---|---|---|---|---|---|");
    for r in rows {
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            r.engine, r.probe, r.wrote, r.write, r.read, r.verdict
        );
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<a panic with no message>".to_string()
    }
}

macro_rules! probe {
    ($rows:expr, $engine:expr, $name:expr, $body:expr) => {{
        let engine: &str = $engine;
        let name: &str = $name;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body));
        $rows.push(match outcome {
            Ok(row) => row,
            Err(payload) => Row::panicked(engine, name, panic_text(&*payload)),
        });
    }};
}

/// Every engine this build enabled, named explicitly - the crate's default
/// features include `redb`, so a store built without naming a backend runs on
/// redb whatever `--features json` said.
fn engines() -> Vec<(&'static str, Backend)> {
    common::enabled_backends()
        .into_iter()
        .map(|backend| (common::engine_name(backend), backend))
        .collect()
}

/// A store with the debouncer and the watcher pushed out of the way, so only
/// `save_now` and the drop write anything.
fn open(file: &TempPath, backend: Backend) -> Result<Store, String> {
    StoreBuilder::new(file.path())
        .backend(backend)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .map_err(|e| brief(&format!("{e:#}")))
}

fn at() -> StorePath {
    StorePath::try_from_segments(["probe", "leaf"]).unwrap()
}

/// Write `W`, flush, drop, reopen, read as `R`, and say what came back.
///
/// Comparison is on the `Debug` rendering rather than on `PartialEq`, so a
/// float that lost its last bits reports as itself.
fn cross<W, R>(engine: &str, backend: Backend, probe: &str, value: &W) -> Row
where
    W: Serialize + std::fmt::Debug,
    R: DeserializeOwned + std::fmt::Debug,
{
    let wrote = format!("{value:?}");
    let file = TempPath::new("probe_serde");
    let path = at();

    let write = {
        let store = match open(&file, backend) {
            Ok(store) => store,
            Err(e) => {
                return Row {
                    engine: engine.to_string(),
                    probe: probe.to_string(),
                    wrote: brief(&wrote),
                    write: "-".to_string(),
                    read: "-".to_string(),
                    verdict: format!("a fresh store would not open: {e}"),
                };
            }
        };

        match store.set(&path, value) {
            Err(e) => {
                return Row {
                    engine: engine.to_string(),
                    probe: probe.to_string(),
                    wrote: brief(&wrote),
                    write: format!("Err: {}", brief(&format!("{e:#}"))),
                    read: "-".to_string(),
                    verdict: "refused on write".to_string(),
                };
            }
            Ok(()) => match store.save_now() {
                Ok(()) => "Ok".to_string(),
                Err(e) => format!("Ok, flush Err: {}", brief(&format!("{e:#}"))),
            },
        }
    };

    let store = match open(&file, backend) {
        Ok(store) => store,
        Err(e) => {
            return Row {
                engine: engine.to_string(),
                probe: probe.to_string(),
                wrote: brief(&wrote),
                write,
                read: "-".to_string(),
                verdict: format!("OPAQUE FAILURE, reopen: {e}"),
            };
        }
    };

    let (read, verdict) = match store.get::<R>(&path) {
        Err(e) => (
            format!("Err: {}", brief(&format!("{e:#}"))),
            "SILENT ALTERATION: written Ok, unreadable".to_string(),
        ),
        Ok(None) => (
            "None".to_string(),
            "SILENT ALTERATION: nothing at the path".to_string(),
        ),
        Ok(Some(back)) => {
            let back = format!("{back:?}");
            let same = back == wrote;
            (
                brief(&back),
                if same {
                    "ok".to_string()
                } else {
                    "SILENT ALTERATION: a different value".to_string()
                },
            )
        }
    };

    Row {
        engine: engine.to_string(),
        probe: probe.to_string(),
        wrote: brief(&wrote),
        write,
        read,
        verdict,
    }
}

/// The same round trip with one type on both ends.
fn round_trip<T>(engine: &str, backend: Backend, probe: &str, value: &T) -> Row
where
    T: Serialize + DeserializeOwned + std::fmt::Debug,
{
    cross::<T, T>(engine, backend, probe, value)
}

/// A round trip that also asks whether the value's own field names became
/// paths of their own under the one that was written.
fn residue_probe<T, D>(
    engine: &str,
    backend: Backend,
    probe: &str,
    value: &T,
    inner: &[&str],
) -> Row
where
    T: Serialize + DeserializeOwned + std::fmt::Debug,
    D: DeserializeOwned + std::fmt::Debug,
{
    let wrote = format!("{value:?}");
    let file = TempPath::new("probe_serde_res");
    let path = at();

    let write = {
        let store = match open(&file, backend) {
            Ok(store) => store,
            Err(e) => {
                return Row {
                    engine: engine.to_string(),
                    probe: probe.to_string(),
                    wrote: brief(&wrote),
                    write: "-".to_string(),
                    read: "-".to_string(),
                    verdict: format!("a fresh store would not open: {e}"),
                };
            }
        };
        match store.set(&path, value) {
            Err(e) => {
                return Row {
                    engine: engine.to_string(),
                    probe: probe.to_string(),
                    wrote: brief(&wrote),
                    write: format!("Err: {}", brief(&format!("{e:#}"))),
                    read: "-".to_string(),
                    verdict: "refused on write".to_string(),
                };
            }
            Ok(()) => match store.save_now() {
                Ok(()) => "Ok".to_string(),
                Err(e) => format!("Ok, flush Err: {}", brief(&format!("{e:#}"))),
            },
        }
    };

    let store = match open(&file, backend) {
        Ok(store) => store,
        Err(e) => {
            return Row {
                engine: engine.to_string(),
                probe: probe.to_string(),
                wrote: brief(&wrote),
                write,
                read: "-".to_string(),
                verdict: format!("OPAQUE FAILURE, reopen: {e}"),
            };
        }
    };

    let mut deep = path.clone();
    let mut buildable = true;
    for level in inner {
        match deep.try_push(level) {
            Ok(next) => deep = next,
            Err(_) => {
                buildable = false;
                break;
            }
        }
    }

    let inner_read = if buildable {
        match store.get::<D>(&deep) {
            Ok(Some(v)) => format!("{deep} = Some({v:?})"),
            Ok(None) => format!("{deep} = None"),
            Err(e) => format!("{deep} = Err: {}", brief(&format!("{e:#}"))),
        }
    } else {
        format!("{inner:?} is not a path this library can build")
    };

    let scanned = match store.scan_keys(&path) {
        Ok(keys) => format!(
            "[{}]",
            keys.iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
    };

    let whole = match store.get::<T>(&path) {
        Ok(Some(v)) => format!("{v:?}"),
        Ok(None) => "None".to_string(),
        Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
    };

    let reachable = buildable && !inner_read.contains("= None") && !inner_read.contains("= Err");

    let verdict = if whole != wrote {
        "SILENT ALTERATION: a different value at the written path".to_string()
    } else if reachable {
        "RESIDUE: the serde name is a path of its own, and nobody wrote it".to_string()
    } else {
        "ok: the serde name stays inside the value".to_string()
    };

    Row {
        engine: engine.to_string(),
        probe: probe.to_string(),
        wrote: brief(&wrote),
        write,
        read: brief(&format!("whole={whole} inner={inner_read} scan={scanned}")),
        verdict,
    }
}

/// A field the codec writes under a name that is not the Rust one.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Renamed {
    #[serde(rename = "stored")]
    declared: u32,
}

/// Every field renamed at once by a naming convention.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RenamedAll {
    my_field: u32,
    another_one: String,
}

/// Written under the old name.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AliasWriter {
    old: u32,
}

/// Read back under the new one, with the old accepted.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AliasReader {
    #[serde(alias = "old")]
    new: u32,
}

/// A serde name holding the character a path uses to separate its levels.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct DottedName {
    #[serde(rename = "a.b")]
    value: u32,
}

/// A serde name holding the escape a path uses.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct EscapedName {
    #[serde(rename = "a\\b")]
    value: u32,
}

/// A serde name that is no name at all.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct EmptyName {
    #[serde(rename = "")]
    value: u32,
}

/// Two fields whose serde names differ only by the level boundary they imply.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct CollidingNames {
    #[serde(rename = "a.b")]
    dotted: u32,
    a: Inner,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Inner {
    b: u32,
}

#[test]
fn attributes_that_change_a_name() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(
            rows,
            engine,
            "rename",
            round_trip(engine, backend, "rename", &Renamed { declared: 7 })
        );
        probe!(
            rows,
            engine,
            "rename, is the serde name a level?",
            residue_probe::<Renamed, u32>(
                engine,
                backend,
                "rename, is the serde name a level?",
                &Renamed { declared: 7 },
                &["stored"],
            )
        );
        probe!(
            rows,
            engine,
            "rename, is the Rust name a level?",
            residue_probe::<Renamed, u32>(
                engine,
                backend,
                "rename, is the Rust name a level?",
                &Renamed { declared: 7 },
                &["declared"],
            )
        );
        probe!(
            rows,
            engine,
            "rename_all camelCase",
            round_trip(
                engine,
                backend,
                "rename_all camelCase",
                &RenamedAll {
                    my_field: 3,
                    another_one: "x".to_string(),
                }
            )
        );
        probe!(
            rows,
            engine,
            "alias: written `old`, read `new`",
            cross::<AliasWriter, AliasReader>(
                engine,
                backend,
                "alias: written `old`, read `new`",
                &AliasWriter { old: 11 }
            )
        );
        probe!(
            rows,
            engine,
            "rename to `a.b`",
            round_trip(engine, backend, "rename to `a.b`", &DottedName { value: 5 })
        );
        probe!(
            rows,
            engine,
            "rename to `a.b`, read as two levels",
            residue_probe::<DottedName, u32>(
                engine,
                backend,
                "rename to `a.b`, read as two levels",
                &DottedName { value: 5 },
                &["a", "b"],
            )
        );
        probe!(
            rows,
            engine,
            "rename to `a\\b`",
            round_trip(
                engine,
                backend,
                "rename to `a\\b`",
                &EscapedName { value: 5 }
            )
        );
        probe!(
            rows,
            engine,
            "rename to the empty name",
            round_trip(
                engine,
                backend,
                "rename to the empty name",
                &EmptyName { value: 5 }
            )
        );
        probe!(
            rows,
            engine,
            "rename to the empty name, as a level",
            residue_probe::<EmptyName, u32>(
                engine,
                backend,
                "rename to the empty name, as a level",
                &EmptyName { value: 5 },
                &[""],
            )
        );
        probe!(
            rows,
            engine,
            "`a.b` beside a real `a` holding `b`",
            residue_probe::<CollidingNames, u32>(
                engine,
                backend,
                "`a.b` beside a real `a` holding `b`",
                &CollidingNames {
                    dotted: 1,
                    a: Inner { b: 2 },
                },
                &["a", "b"],
            )
        );
    }

    table("attributes that change a name", &rows);
}

/// What a serde name costs when the store treats it as a level of its own.
///
/// Two directions: an unrelated write at the path the serde name occupies, and
/// a value assembled out of paths nobody wrote as a value.
#[test]
fn a_serde_name_and_a_path_meet() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(rows, engine, "a write at the serde name's path", {
            let probe = "a write at the serde name's path";
            let file = TempPath::new("probe_serde_collide");
            let path = at();
            let inner = path.try_push("stored").unwrap();

            {
                let store = open(&file, backend).unwrap();
                store.set(&path, &Renamed { declared: 7 }).unwrap();
                store.set(&inner, &99u32).unwrap();
                store.save_now().unwrap();
            }

            let store = open(&file, backend).unwrap();
            let whole = match store.get::<Renamed>(&path) {
                Ok(Some(v)) => format!("{v:?}"),
                Ok(None) => "None".to_string(),
                Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
            };

            let verdict = if whole.contains("declared: 99") {
                "SILENT ALTERATION: a write at another path replaced a field inside the value"
                    .to_string()
            } else if whole.contains("declared: 7") {
                "ok: the two writes do not reach each other".to_string()
            } else {
                format!("neither value survived: {whole}")
            };

            Row {
                engine: engine.to_string(),
                probe: probe.to_string(),
                wrote: "Renamed { declared: 7 } at probe.leaf, then 99u32 at probe.leaf.stored"
                    .to_string(),
                write: "Ok".to_string(),
                read: brief(&format!("probe.leaf as Renamed = {whole}")),
                verdict,
            }
        });

        probe!(
            rows,
            engine,
            "a value nobody wrote, assembled from a path",
            {
                let probe = "a value nobody wrote, assembled from a path";
                let file = TempPath::new("probe_serde_assemble");
                let path = at();
                let inner = path.try_push("stored").unwrap();

                {
                    let store = open(&file, backend).unwrap();
                    store.set(&inner, &42u32).unwrap();
                    store.save_now().unwrap();
                }

                let store = open(&file, backend).unwrap();
                let whole = match store.get::<Renamed>(&path) {
                    Ok(Some(v)) => format!("{v:?}"),
                    Ok(None) => "None".to_string(),
                    Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
                };

                let verdict = if whole.contains("declared: 42") {
                    "RESIDUE: a struct nobody wrote reads back, built out of one scalar".to_string()
                } else if whole == "None" {
                    "ok: only what was written is a value".to_string()
                } else {
                    "the level above the write is a value of its own, and not the struct's"
                        .to_string()
                };

                Row {
                    engine: engine.to_string(),
                    probe: probe.to_string(),
                    wrote: "42u32 at probe.leaf.stored, and nothing at probe.leaf".to_string(),
                    write: "Ok".to_string(),
                    read: brief(&format!("probe.leaf as Renamed = {whole}")),
                    verdict,
                }
            }
        );
    }

    table("a serde name and a path meet", &rows);
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct FlatInner {
    deep: String,
    number: u64,
}

/// `flatten` puts an inner struct's fields at the outer level, and forces the
/// serializer into map-collecting mode on the way.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Flattened {
    top: u32,
    #[serde(flatten)]
    inner: FlatInner,
}

/// `flatten` over a map, which is the catch-all shape.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct FlatRest {
    known: u32,
    #[serde(flatten)]
    rest: BTreeMap<String, u64>,
}

/// A number `flatten`'s buffering has nowhere to keep.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct FlatBig {
    #[serde(flatten)]
    inner: BigHolder,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct BigHolder {
    big: u128,
}

/// The wrapper serialises as the thing it wraps.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(transparent)]
struct Transparent {
    value: Vec<u32>,
}

/// The stored shape is a different type than the one declared.
#[derive(Debug, PartialEq, Clone)]
struct Celsius(f64);

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AsText(String);

impl From<Celsius> for AsText {
    fn from(c: Celsius) -> Self {
        AsText(format!("{}C", c.0))
    }
}

impl From<AsText> for Celsius {
    fn from(t: AsText) -> Self {
        let digits = t.0.trim_end_matches('C');
        Celsius(digits.parse().unwrap_or(f64::NAN))
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(from = "AsText", into = "AsText")]
struct Temperature(Celsius);

impl Clone for Temperature {
    fn clone(&self) -> Self {
        Temperature(self.0.clone())
    }
}

impl From<AsText> for Temperature {
    fn from(t: AsText) -> Self {
        Temperature(Celsius::from(t))
    }
}

impl From<Temperature> for AsText {
    fn from(t: Temperature) -> Self {
        AsText::from(t.0)
    }
}

/// `into` writes a shape `try_from` will refuse: a write that returns `Ok` and
/// can never be read.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
struct Bounded(u32);

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(try_from = "u32", into = "u32")]
struct SmallOnly(u32);

impl TryFrom<u32> for SmallOnly {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        if v < 10 {
            Ok(SmallOnly(v))
        } else {
            Err(format!("{v} is out of range"))
        }
    }
}

impl From<SmallOnly> for u32 {
    fn from(v: SmallOnly) -> u32 {
        v.0 * 100
    }
}

#[test]
fn attributes_that_change_the_shape() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(
            rows,
            engine,
            "flatten a struct",
            round_trip(
                engine,
                backend,
                "flatten a struct",
                &Flattened {
                    top: 1,
                    inner: FlatInner {
                        deep: "d".to_string(),
                        number: 2,
                    },
                }
            )
        );
        probe!(
            rows,
            engine,
            "flatten, is the inner field a level?",
            residue_probe::<Flattened, String>(
                engine,
                backend,
                "flatten, is the inner field a level?",
                &Flattened {
                    top: 1,
                    inner: FlatInner {
                        deep: "d".to_string(),
                        number: 2,
                    },
                },
                &["deep"],
            )
        );
        probe!(
            rows,
            engine,
            "flatten, is `inner` itself a level?",
            residue_probe::<Flattened, FlatInner>(
                engine,
                backend,
                "flatten, is `inner` itself a level?",
                &Flattened {
                    top: 1,
                    inner: FlatInner {
                        deep: "d".to_string(),
                        number: 2,
                    },
                },
                &["inner"],
            )
        );
        probe!(
            rows,
            engine,
            "flatten a map",
            round_trip(
                engine,
                backend,
                "flatten a map",
                &FlatRest {
                    known: 1,
                    rest: BTreeMap::from([("x".to_string(), 9u64)]),
                }
            )
        );
        probe!(
            rows,
            engine,
            "control: u128::MAX with no flatten",
            round_trip(
                engine,
                backend,
                "control: u128::MAX with no flatten",
                &BigHolder { big: u128::MAX }
            )
        );
        probe!(
            rows,
            engine,
            "control: 2^63 with no flatten",
            round_trip(
                engine,
                backend,
                "control: 2^63 with no flatten",
                &BigHolder {
                    big: 9_223_372_036_854_775_808u128,
                }
            )
        );
        probe!(
            rows,
            engine,
            "flatten holding u128::MAX",
            round_trip(
                engine,
                backend,
                "flatten holding u128::MAX",
                &FlatBig {
                    inner: BigHolder { big: u128::MAX },
                }
            )
        );
        probe!(
            rows,
            engine,
            "flatten holding 2^63",
            round_trip(
                engine,
                backend,
                "flatten holding 2^63",
                &FlatBig {
                    inner: BigHolder {
                        big: 9_223_372_036_854_775_808u128,
                    },
                }
            )
        );
        probe!(
            rows,
            engine,
            "transparent",
            round_trip(
                engine,
                backend,
                "transparent",
                &Transparent {
                    value: vec![1, 2, 3],
                }
            )
        );
        probe!(
            rows,
            engine,
            "from/into a different type",
            round_trip(
                engine,
                backend,
                "from/into a different type",
                &Temperature(Celsius(21.5))
            )
        );
        probe!(
            rows,
            engine,
            "into writes what try_from refuses",
            round_trip(
                engine,
                backend,
                "into writes what try_from refuses",
                &SmallOnly(1)
            )
        );
    }

    table("attributes that change the shape", &rows);
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
enum Internally {
    Alpha { name: String },
    Beta { count: u32 },
}

/// Internally tagged, over data that can carry the tag's own name.
///
/// A *declared* field whose serde name is the tag's is a compile error - serde
/// says "variant field name `kind` conflicts with internal tag" - so the
/// collision has to arrive at runtime, through a flattened map.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind")]
enum TagCollision {
    One {
        #[serde(flatten)]
        rest: BTreeMap<String, String>,
    },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "t", content = "c")]
enum Adjacent {
    Wrapped(u64),
    Structured { a: u32 },
}

/// Two arms whose data overlaps: a `Big` and a `Small` that hold the same
/// number.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum Overlapping {
    Small(u8),
    Big(u64),
}

/// The same, declared the other way round.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum OverlappingReversed {
    Big(u64),
    Small(u8),
}

/// Two struct arms where one's fields are a subset of the other's.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum SubsetArms {
    Narrow { a: u32 },
    Wide { a: u32, b: u32 },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum WithOther {
    Known,
    #[serde(other)]
    Anything,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum OnlyKnown {
    Known,
    Retired,
}

/// The control every enum probe is read against: no serde attribute at all, so
/// whatever it does is the engine's plain treatment of an enum.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum Plain {
    Unit,
    Newtype(u32),
    Structured { a: u32 },
}

#[test]
fn enums() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(
            rows,
            engine,
            "control: a plain unit variant",
            round_trip(
                engine,
                backend,
                "control: a plain unit variant",
                &Plain::Unit
            )
        );
        probe!(
            rows,
            engine,
            "control: a plain newtype variant",
            round_trip(
                engine,
                backend,
                "control: a plain newtype variant",
                &Plain::Newtype(5)
            )
        );
        probe!(
            rows,
            engine,
            "control: a plain struct variant",
            round_trip(
                engine,
                backend,
                "control: a plain struct variant",
                &Plain::Structured { a: 5 }
            )
        );
        probe!(
            rows,
            engine,
            "internally tagged",
            round_trip(
                engine,
                backend,
                "internally tagged",
                &Internally::Beta { count: 4 }
            )
        );
        probe!(
            rows,
            engine,
            "internally tagged, is the tag a level?",
            residue_probe::<Internally, String>(
                engine,
                backend,
                "internally tagged, is the tag a level?",
                &Internally::Alpha {
                    name: "n".to_string()
                },
                &["type"],
            )
        );
        probe!(
            rows,
            engine,
            "the tag collides with real data",
            round_trip(
                engine,
                backend,
                "the tag collides with real data",
                &TagCollision::One {
                    rest: BTreeMap::from([("kind".to_string(), "data".to_string())]),
                }
            )
        );
        probe!(
            rows,
            engine,
            "adjacently tagged, newtype arm",
            round_trip(
                engine,
                backend,
                "adjacently tagged, newtype arm",
                &Adjacent::Wrapped(9)
            )
        );
        probe!(
            rows,
            engine,
            "adjacently tagged, struct arm",
            round_trip(
                engine,
                backend,
                "adjacently tagged, struct arm",
                &Adjacent::Structured { a: 1 }
            )
        );
        probe!(
            rows,
            engine,
            "untagged Big(5), Small declared first",
            round_trip(
                engine,
                backend,
                "untagged Big(5), Small declared first",
                &Overlapping::Big(5)
            )
        );
        probe!(
            rows,
            engine,
            "untagged Small(5), Small declared first",
            round_trip(
                engine,
                backend,
                "untagged Small(5), Small declared first",
                &Overlapping::Small(5)
            )
        );
        probe!(
            rows,
            engine,
            "untagged Big(5), Big declared first",
            round_trip(
                engine,
                backend,
                "untagged Big(5), Big declared first",
                &OverlappingReversed::Big(5)
            )
        );
        probe!(
            rows,
            engine,
            "untagged Big(300), out of Small's range",
            round_trip(
                engine,
                backend,
                "untagged Big(300), out of Small's range",
                &Overlapping::Big(300)
            )
        );
        probe!(
            rows,
            engine,
            "untagged Wide, Narrow's fields are a subset",
            round_trip(
                engine,
                backend,
                "untagged Wide, Narrow's fields are a subset",
                &SubsetArms::Wide { a: 1, b: 2 }
            )
        );
        probe!(
            rows,
            engine,
            "other: a retired variant reads as the catch-all",
            cross::<OnlyKnown, WithOther>(
                engine,
                backend,
                "other: a retired variant reads as the catch-all",
                &OnlyKnown::Retired
            )
        );
        probe!(
            rows,
            engine,
            "other: a known variant still reads as itself",
            cross::<OnlyKnown, WithOther>(
                engine,
                backend,
                "other: a known variant still reads as itself",
                &OnlyKnown::Known
            )
        );
    }

    table("enums", &rows);
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Skipped {
    kept: u32,
    #[serde(skip)]
    hidden: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
struct SkipSerializingOnly {
    kept: u32,
    #[serde(skip_serializing)]
    write_only: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
struct SkipDeserializingOnly {
    kept: u32,
    #[serde(skip_deserializing)]
    read_never: u32,
}

/// Two fields of the same type where the first is conditionally absent, which
/// is where a positional encoding shifts one value into the other's place.
#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
struct ConditionallyAbsent {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    maybe: Option<u32>,
    always: u32,
}

/// The same shape without the `default`, so a lost field is an error rather
/// than a silence.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AbsentWithoutDefault {
    #[serde(skip_serializing_if = "Option::is_none")]
    maybe: Option<u32>,
    always: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct V1 {
    a: u32,
}

/// The next version of `V1`, whose new field defaults - so a value the store
/// lost looks exactly like one that was never set.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct V2Defaulting {
    a: u32,
    #[serde(default = "eight")]
    b: u32,
}

fn eight() -> u32 {
    8
}

/// The next version of `V1` that refuses anything it does not know.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct StrictV1 {
    a: u32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct V2Wider {
    a: u32,
    b: u32,
}

#[test]
fn attributes_that_hide_a_value() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(
            rows,
            engine,
            "skip",
            round_trip(
                engine,
                backend,
                "skip",
                &Skipped {
                    kept: 1,
                    hidden: 42
                }
            )
        );
        probe!(
            rows,
            engine,
            "skip_serializing",
            round_trip(
                engine,
                backend,
                "skip_serializing",
                &SkipSerializingOnly {
                    kept: 1,
                    write_only: 42,
                }
            )
        );
        probe!(
            rows,
            engine,
            "skip_deserializing",
            round_trip(
                engine,
                backend,
                "skip_deserializing",
                &SkipDeserializingOnly {
                    kept: 1,
                    read_never: 42,
                }
            )
        );
        probe!(
            rows,
            engine,
            "skip_serializing_if, first of two, with default",
            round_trip(
                engine,
                backend,
                "skip_serializing_if, first of two, with default",
                &ConditionallyAbsent {
                    maybe: None,
                    always: 77,
                }
            )
        );
        probe!(
            rows,
            engine,
            "skip_serializing_if, first of two, no default",
            round_trip(
                engine,
                backend,
                "skip_serializing_if, first of two, no default",
                &AbsentWithoutDefault {
                    maybe: None,
                    always: 77,
                }
            )
        );
        probe!(
            rows,
            engine,
            "default masks a field the file never held",
            cross::<V1, V2Defaulting>(
                engine,
                backend,
                "default masks a field the file never held",
                &V1 { a: 1 }
            )
        );
        probe!(
            rows,
            engine,
            "deny_unknown_fields meets a field the type dropped",
            cross::<V2Wider, StrictV1>(
                engine,
                backend,
                "deny_unknown_fields meets a field the type dropped",
                &V2Wider { a: 1, b: 2 }
            )
        );
    }

    table("attributes that hide a value", &rows);
}

/// Writes a map where the declared type is a scalar, and reads a scalar back.
mod map_for_a_scalar {
    use super::*;

    pub fn serialize<S: Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry("value", v)?;
        m.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        u32::deserialize(d)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AsymmetricCode {
    #[serde(with = "map_for_a_scalar")]
    value: u32,
}

/// A sequence whose length is decided when it is written, and re-decided the
/// next time the same value is written.
static LENGTHS: AtomicU32 = AtomicU32::new(0);

mod growing_sequence {
    use super::*;

    pub fn serialize<S: Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
        let n = LENGTHS.fetch_add(1, Ordering::SeqCst) + 1;
        let items: Vec<u32> = (0..n).map(|_| *v).collect();
        items.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        let items = Vec::<u32>::deserialize(d)?;
        Ok(items.len() as u32)
    }
}

/// The count is what the value's own `Serialize` was asked for, so what lands
/// says how many times the store ran it.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct CountsItsWrites {
    #[serde(with = "growing_sequence")]
    value: u32,
}

/// A `Serialize` that answers differently depending on whether the format
/// calls itself human-readable.
#[derive(Debug, PartialEq)]
struct TwoFaced(u32);

impl Serialize for TwoFaced {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&self.0.to_string())
        } else {
            self.0.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for TwoFaced {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            let s = String::deserialize(d)?;
            s.parse().map(TwoFaced).map_err(serde::de::Error::custom)
        } else {
            u32::deserialize(d).map(TwoFaced)
        }
    }
}

/// Serialises three fields and deserialises two - a `Serialize` that is not
/// symmetric with its `Deserialize`.
#[derive(Debug, PartialEq)]
struct Lopsided {
    a: u32,
    b: u32,
}

impl Serialize for Lopsided {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(3))?;
        m.serialize_entry("a", &self.a)?;
        m.serialize_entry("b", &self.b)?;
        m.serialize_entry("c", &"an extra nobody reads")?;
        m.end()
    }
}

impl<'de> Deserialize<'de> for Lopsided {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Shape {
            a: u32,
            b: u32,
        }
        let s = Shape::deserialize(d)?;
        Ok(Lopsided { a: s.a, b: s.b })
    }
}

#[test]
fn attributes_that_hand_serde_arbitrary_code() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(
            rows,
            engine,
            "with: writes a map, reads a scalar",
            round_trip(
                engine,
                backend,
                "with: writes a map, reads a scalar",
                &AsymmetricCode { value: 4 }
            )
        );
        probe!(rows, engine, "serialize_with counting its own calls", {
            LENGTHS.store(0, Ordering::SeqCst);
            let row = round_trip(
                engine,
                backend,
                "serialize_with counting its own calls",
                &CountsItsWrites { value: 1 },
            );
            Row {
                read: format!(
                    "{} (Serialize ran {} times for one set)",
                    row.read,
                    LENGTHS.load(Ordering::SeqCst)
                ),
                ..row
            }
        });
        probe!(
            rows,
            engine,
            "a Serialize that branches on is_human_readable",
            round_trip(
                engine,
                backend,
                "a Serialize that branches on is_human_readable",
                &TwoFaced(5)
            )
        );
        probe!(
            rows,
            engine,
            "writes three fields, reads two, denies the third",
            round_trip(
                engine,
                backend,
                "writes three fields, reads two, denies the third",
                &Lopsided { a: 1, b: 2 }
            )
        );
    }

    table("attributes that hand serde arbitrary code", &rows);
}

/// A declared struct whose fields carry serde names beside their Rust ones.
///
/// The macro reads `amestate` and nothing else, so whether the serde name ever
/// reaches the store, the schema or both is what this asks.
#[amethystate(prefix = "serde_named", version = 1)]
pub struct SerdeNamed {
    #[serde(rename = "renamed_port")]
    #[amestate(default = 8080)]
    pub port: u16,

    #[amestate(key = "keyed_host", default = "h".to_string())]
    #[serde(rename = "serde_host")]
    pub host: String,
}

#[test]
fn serde_names_on_a_declared_struct() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(rows, engine, "declared struct with serde renames", {
            let file = TempPath::new("probe_serde_decl");

            {
                let store = match open(&file, backend) {
                    Ok(store) => store,
                    Err(e) => {
                        return Row {
                            engine: engine.to_string(),
                            probe: "declared struct with serde renames".to_string(),
                            wrote: "-".to_string(),
                            write: "-".to_string(),
                            read: "-".to_string(),
                            verdict: format!("a fresh store would not open: {e}"),
                        };
                    }
                };

                let state = SerdeNamed::new_with(&store).unwrap();
                state.port().set(9999).unwrap();
                state.host().set("written".to_string()).unwrap();
                store.save_now().unwrap();
            }

            let store = open(&file, backend).unwrap();

            let by_rust =
                store.get::<u16>(StorePath::try_from_segments(["serde_named", "port"]).unwrap());
            let by_serde = store
                .get::<u16>(StorePath::try_from_segments(["serde_named", "renamed_port"]).unwrap());
            let by_key = store.get::<String>(
                StorePath::try_from_segments(["serde_named", "keyed_host"]).unwrap(),
            );
            let by_serde_host = store.get::<String>(
                StorePath::try_from_segments(["serde_named", "serde_host"]).unwrap(),
            );

            let keys = store
                .scan_keys(StorePath::segment("serde_named"))
                .map(|k| {
                    k.iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|e| format!("Err: {}", brief(&format!("{e:#}"))));

            let render = |v: &Result<Option<u16>, _>| match v {
                Ok(Some(v)) => format!("Some({v})"),
                Ok(None) => "None".to_string(),
                Err(_) => "Err".to_string(),
            };
            let render_s = |v: &Result<Option<String>, _>| match v {
                Ok(Some(v)) => format!("Some({v:?})"),
                Ok(None) => "None".to_string(),
                Err(_) => "Err".to_string(),
            };

            let verdict = match (&by_rust, &by_serde) {
                (Ok(Some(_)), Ok(None)) => {
                    "the Rust name won; the serde name named nothing".to_string()
                }
                (Ok(None), Ok(Some(_))) => "the serde name won".to_string(),
                (Ok(Some(_)), Ok(Some(_))) => "RESIDUE: both names hold a value".to_string(),
                _ => "neither name holds a value".to_string(),
            };

            Row {
                engine: engine.to_string(),
                probe: "declared struct with serde renames".to_string(),
                wrote: "port=9999 host=\"written\"".to_string(),
                write: "Ok".to_string(),
                read: brief(&format!(
                    "port={} renamed_port={} keyed_host={} serde_host={} scan=[{}]",
                    render(&by_rust),
                    render(&by_serde),
                    render_s(&by_key),
                    render_s(&by_serde_host),
                    keys
                )),
                verdict,
            }
        });
    }

    table("serde names on a declared struct", &rows);
}

/// What the file says the declared struct is, beside what it holds.
#[test]
fn the_schema_snapshot_beside_the_data() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(rows, engine, "snapshot names vs stored paths", {
            let file = TempPath::new("probe_serde_schema");

            {
                let store = open(&file, backend).unwrap();
                let state = SerdeNamed::new_with(&store).unwrap();
                state.port().set(9999).unwrap();
                store.save_now().unwrap();
            }

            let snapshot = snapshot_names(&file, backend);

            let store = open(&file, backend).unwrap();
            let stored: Vec<String> = store
                .scan_keys(StorePath::segment("serde_named"))
                .map(|keys| {
                    keys.iter()
                        .filter_map(|k| k.segments().last().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let verdict = match &snapshot {
                None => "no snapshot could be read on this engine".to_string(),
                Some(names) => {
                    let unmatched: Vec<&String> =
                        names.iter().filter(|n| !stored.contains(n)).collect();
                    let unnamed: Vec<&String> =
                        stored.iter().filter(|s| !names.contains(s)).collect();

                    if unmatched.is_empty() && unnamed.is_empty() {
                        "the snapshot names exactly the paths that hold the data".to_string()
                    } else {
                        format!(
                            "SCHEMA DISAGREEMENT: the snapshot names {unmatched:?}, \
                             which hold nothing; the data is at {unnamed:?}, \
                             which the snapshot does not name"
                        )
                    }
                }
            };

            Row {
                engine: engine.to_string(),
                probe: "snapshot names vs stored paths".to_string(),
                wrote: "port=9999, host defaulted".to_string(),
                write: "Ok".to_string(),
                read: brief(&format!("snapshot={snapshot:?} stored={stored:?}")),
                verdict,
            }
        });
    }

    table("the schema snapshot beside the data", &rows);
}

/// The names the file's own schema snapshot records for `serde_named`.
fn snapshot_names(file: &TempPath, backend: Backend) -> Option<Vec<String>> {
    use amethystate::observability::InspectorBackend;
    use amethystate::store::config::StoreConfig;

    let snapshots = match backend {
        #[cfg(feature = "redb")]
        Backend::Redb => {
            amethystate::stores::RedbStore::open(StoreConfig::new(file.path()), Default::default())
                .ok()?
                .0
                .get_schema_snapshots()
                .ok()?
        }
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => amethystate::stores::SqliteStore::open(
            StoreConfig::new(file.path()),
            Default::default(),
        )
        .ok()?
        .0
        .get_schema_snapshots()
        .ok()?,
        #[cfg(feature = "json")]
        Backend::Json => {
            amethystate::stores::JsonStore::open(StoreConfig::new(file.path()), Default::default())
                .ok()?
                .0
                .get_schema_snapshots()
                .ok()?
        }
        #[cfg(feature = "toml")]
        Backend::Toml => {
            amethystate::stores::TomlStore::open(StoreConfig::new(file.path()), Default::default())
                .ok()?
                .0
                .get_schema_snapshots()
                .ok()?
        }
        #[cfg(feature = "ron")]
        Backend::Ron => {
            amethystate::stores::RonStore::open(StoreConfig::new(file.path()), Default::default())
                .ok()?
                .0
                .get_schema_snapshots()
                .ok()?
        }
    };

    snapshots
        .into_iter()
        .find(|(prefix, _)| prefix.contains("serde_named"))
        .map(|(_, snapshot)| snapshot.fields.iter().map(|f| f.name.clone()).collect())
}

/// Whether the shape the inspector could not show is in the file at all.
///
/// A text store keeps its metadata in a `.meta` sibling, so the snapshot can be
/// looked for there directly. This separates "nothing was recorded" from "the
/// inspector cannot read what was recorded".
#[test]
fn the_shape_a_text_store_recorded() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(rows, engine, "the sibling .meta file", {
            let file = TempPath::new("probe_serde_meta");

            {
                let store = open(&file, backend).unwrap();
                let state = SerdeNamed::new_with(&store).unwrap();
                state.port().set(9999).unwrap();
                store.save_now().unwrap();
            }

            let meta = std::fs::read_to_string(file.path().with_extension("meta"))
                .unwrap_or_else(|e| format!("<unreadable: {e}>"));

            let inspector = snapshot_names(&file, backend);

            let in_file = meta.contains("serde_named");
            let verdict = match (in_file, &inspector) {
                (true, Some(_)) => {
                    "the file holds the shape and the inspector shows it".to_string()
                }
                (true, None) => "SCHEMA DISAGREEMENT: the file holds the shape and the \
                                 inspector reports none"
                    .to_string(),
                (false, Some(_)) => "the inspector shows a shape the .meta file does not hold \
                                     - this engine keeps it elsewhere"
                    .to_string(),
                (false, None) => "no .meta file, and no snapshot from the inspector".to_string(),
            };

            Row {
                engine: engine.to_string(),
                probe: "the sibling .meta file".to_string(),
                wrote: "port=9999".to_string(),
                write: "Ok".to_string(),
                read: brief(&format!("inspector={inspector:?} meta={meta}")),
                verdict,
            }
        });
    }

    table("the shape a text store recorded", &rows);
}

/// Nested arrays, one level per `Nest`.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Nest(Vec<Nest>);

fn nest(levels: usize) -> Nest {
    let mut out = Nest(Vec::new());
    for _ in 1..levels {
        out = Nest(vec![out]);
    }
    out
}

fn depth_of(n: &Nest) -> usize {
    1 + n.0.first().map(depth_of).unwrap_or(0)
}

/// A scalar to a human-readable format and a deep nest to a binary one.
///
/// The store measures a value's depth by pushing it through a counting
/// serializer before the codec sees it, and that counter answers
/// `is_human_readable` for itself. A value that branches on the answer is
/// therefore measured in one shape and written in another.
#[derive(Debug, PartialEq)]
struct TwoFacedDepth(usize);

impl Serialize for TwoFacedDepth {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str("flat")
        } else {
            nest(self.0).serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for TwoFacedDepth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            let _ = String::deserialize(d)?;
            Ok(TwoFacedDepth(0))
        } else {
            Nest::deserialize(d).map(|n| TwoFacedDepth(depth_of(&n)))
        }
    }
}

/// The value nests this far in its binary shape, and not at all in its
/// human-readable one.
///
/// Small enough that a codec with no read limit does not take the process down
/// with it - the point is which shape the guard measured, not how far a stack
/// goes.
///
/// The budget is shared between the path and the value, so the probe writes at
/// a path deep enough to leave almost nothing, which is how a store is brought
/// to the edge of its ceiling without reaching for a setting.
const BINARY_LEVELS: usize = 100;

#[test]
fn a_value_measured_in_one_shape_and_written_in_another() {
    let mut rows = Vec::new();

    for (engine, backend) in engines() {
        probe!(
            rows,
            engine,
            "is_human_readable splits the guard from the codec",
            {
                let probe = "is_human_readable splits the guard from the codec";
                let file = TempPath::new("probe_serde_depth");

                let spend = backend.depth_ceiling() - 10;
                let path = StorePath::try_from_segments(
                    (0..spend).map(|i| format!("s{i}")).collect::<Vec<_>>(),
                )
                .unwrap();
                let value = TwoFacedDepth(BINARY_LEVELS);

                let write = {
                    let store = open(&file, backend).unwrap();
                    match store.set(&path, &value) {
                        Err(e) => {
                            return Row {
                                engine: engine.to_string(),
                                probe: probe.to_string(),
                                wrote: format!("{BINARY_LEVELS} levels, or the string \"flat\""),
                                write: format!("Err: {}", brief(&format!("{e:#}"))),
                                read: "-".to_string(),
                                verdict:
                                    "refused on write: the guard measured what the codec writes"
                                        .to_string(),
                            };
                        }
                        Ok(()) => match store.save_now() {
                            Ok(()) => "Ok".to_string(),
                            Err(e) => format!("Ok, flush Err: {}", brief(&format!("{e:#}"))),
                        },
                    }
                };

                let store = match open(&file, backend) {
                    Ok(store) => store,
                    Err(e) => {
                        return Row {
                            engine: engine.to_string(),
                            probe: probe.to_string(),
                            wrote: format!("{BINARY_LEVELS} levels, or the string \"flat\""),
                            write,
                            read: "-".to_string(),
                            verdict: format!("OPAQUE FAILURE, reopen: {e}"),
                        };
                    }
                };

                let (read, landed) = match store.get::<TwoFacedDepth>(&path) {
                    Ok(Some(TwoFacedDepth(n))) => (format!("{n} levels"), n),
                    Ok(None) => ("None".to_string(), 0),
                    Err(e) => (format!("Err: {}", brief(&format!("{e:#}"))), 0),
                };

                let ceiling = backend.depth_ceiling();
                let left = ceiling - spend;
                let verdict = if landed > left {
                    format!(
                        "the guard let {landed} levels through with {left} left in the budget, \
                     because it measured the human-readable shape and this codec is not"
                    )
                } else if landed == 0 {
                    format!(
                        "this codec is human-readable, so the flat shape is the one that was \
                     written ({left} levels were left)"
                    )
                } else {
                    format!("{landed} levels landed, within the {left} that were left")
                };

                Row {
                    engine: engine.to_string(),
                    probe: probe.to_string(),
                    wrote: format!(
                        "{BINARY_LEVELS} levels, or the string \"flat\", at a path spending \
                     {spend} of {ceiling}"
                    ),
                    write,
                    read,
                    verdict,
                }
            }
        );
    }

    table(
        "a value measured in one shape and written in another",
        &rows,
    );
}
