//! How far "the store took it and will not give it back" reaches on json.
//!
//! Every probe here does the same four things: write a value through a store,
//! flush it, drop the store, open the file again, and read the same path back.
//! Nothing is asserted about what *should* happen - the tests print a table and
//! only fail when a probe cannot be run at all. The point is the table.
//!
//! Three verdicts are worth separating:
//!
//! - **opaque failure** - the file will not open again at all.
//! - **silent alteration** - the write returned `Ok` and the read gives back
//!   something else, or nothing, or an error.
//! - **residue** - a path nobody wrote is readable afterwards.

#![cfg(feature = "json")]

use amethystate::Store;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::reactive_map_with_path_only;
use amethystate::uuid::Uuid;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// One line of the report.
struct Row {
    probe: String,
    wrote: String,
    write: String,
    read: String,
    verdict: String,
}

impl Row {
    fn panicked(probe: &str, what: String) -> Self {
        Row {
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

    let mut out: String = flat.chars().take(150).collect();
    if flat.chars().count() > 150 {
        out.push_str(&format!(" …({} chars)", flat.chars().count()));
    }
    out
}

fn table(section: &str, rows: &[Row]) {
    println!("\n## {section}\n");
    println!("| probe | written | write returned | read back | verdict |");
    println!("|---|---|---|---|---|");
    for r in rows {
        println!(
            "| {} | {} | {} | {} | {} |",
            r.probe, r.wrote, r.write, r.read, r.verdict
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
    ($rows:expr, $name:expr, $body:expr) => {{
        let name: &str = $name;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body));
        $rows.push(match outcome {
            Ok(row) => row,
            Err(payload) => Row::panicked(name, panic_text(&*payload)),
        });
    }};
}

/// A json store with the debouncer and the watcher pushed out of the way, so
/// only `save_now` and the drop write anything.
fn open(file: &TempPath) -> Result<Store, String> {
    StoreBuilder::new(file.path())
        .backend(Backend::Json)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .map_err(|e| brief(&format!("{e:#}")))
}

/// Every path in the document that holds no further levels, as its segments.
///
/// An empty object is one of them: it is a node with nothing under it, which is
/// exactly the shape a residue takes.
fn leaf_paths(text: &str) -> Vec<Vec<String>> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(&value, &mut Vec::new(), &mut out);
    out
}

fn walk(value: &serde_json::Value, at: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match value.as_object() {
        Some(map) if !map.is_empty() => {
            for (k, v) in map {
                at.push(k.clone());
                walk(v, at, out);
                at.pop();
            }
        }
        _ => out.push(at.clone()),
    }
}

/// The leaves that are not under the path that was written, rendered.
fn stray(leaves: &[Vec<String>], written: &[&str]) -> Option<String> {
    let strays: Vec<String> = leaves
        .iter()
        .filter(|leaf| leaf.len() < written.len() || !leaf.iter().zip(written).all(|(a, b)| a == b))
        .map(|leaf| format!("{leaf:?}"))
        .collect();

    if strays.is_empty() {
        None
    } else {
        Some(strays.join(" "))
    }
}

/// Write, flush, reopen, read, compare - the whole probe, for a value at a
/// path.
///
/// Comparison is on the `Debug` rendering rather than on `PartialEq`, so that
/// `NaN`, `-0.0` and a float that lost its last bits all report as themselves.
fn round_trip<T>(probe: &str, segments: &[&str], value: &T) -> Row
where
    T: Serialize + DeserializeOwned + std::fmt::Debug,
{
    let wrote = format!("{value:?}");
    let file = TempPath::new("probe_json");

    let path = match StorePath::try_from_segments(segments) {
        Ok(path) => path,
        Err(e) => {
            return Row {
                probe: probe.to_string(),
                wrote: brief(&wrote),
                write: format!("no write: {e}"),
                read: "-".to_string(),
                verdict: "refused before the store saw it".to_string(),
            };
        }
    };

    let write = {
        let store = match open(&file) {
            Ok(store) => store,
            Err(e) => {
                return Row {
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

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let leaves = leaf_paths(&text);

    let store = match open(&file) {
        Ok(store) => store,
        Err(e) => {
            return Row {
                probe: probe.to_string(),
                wrote: brief(&wrote),
                write,
                read: "-".to_string(),
                verdict: format!("OPAQUE FAILURE, reopen: {e}"),
            };
        }
    };

    let (read, verdict) = match store.get::<T>(&path) {
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
            match (same, stray(&leaves, segments)) {
                (true, None) => (brief(&back), "ok".to_string()),
                (true, Some(extra)) => (
                    brief(&back),
                    format!("RESIDUE: the file also holds {}", brief(&extra)),
                ),
                (false, _) => (
                    brief(&back),
                    "SILENT ALTERATION: a different value".to_string(),
                ),
            }
        }
    };

    Row {
        probe: probe.to_string(),
        wrote: brief(&wrote),
        write,
        read,
        verdict,
    }
}

/// The same round trip with the interest on the path rather than the value: a
/// fixed `7u32` under `probe`, read back both by path and by a scan of the
/// level above it.
fn path_probe(probe: &str, segment: &str) -> Row {
    let file = TempPath::new("probe_json_path");
    let parent = StorePath::segment("probe");

    let path = match parent.try_push(segment) {
        Ok(path) => path,
        Err(e) => {
            return Row {
                probe: probe.to_string(),
                wrote: brief(&format!("{segment:?}")),
                write: format!("no write: {e}"),
                read: "-".to_string(),
                verdict: "refused before the store saw it".to_string(),
            };
        }
    };

    let write = {
        let store = match open(&file) {
            Ok(store) => store,
            Err(e) => {
                return Row {
                    probe: probe.to_string(),
                    wrote: brief(&format!("{segment:?}")),
                    write: "-".to_string(),
                    read: "-".to_string(),
                    verdict: format!("a fresh store would not open: {e}"),
                };
            }
        };

        match store.set(&path, &7u32) {
            Err(e) => {
                return Row {
                    probe: probe.to_string(),
                    wrote: brief(&format!("{segment:?}")),
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

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let on_disk = leaf_paths(&text);

    let store = match open(&file) {
        Ok(store) => store,
        Err(e) => {
            return Row {
                probe: probe.to_string(),
                wrote: brief(&format!("{segment:?}")),
                write,
                read: "-".to_string(),
                verdict: format!("OPAQUE FAILURE, reopen: {e}"),
            };
        }
    };

    let got = store.get::<u32>(&path);
    let scanned = store.scan_keys(&parent);

    let scan_render = match &scanned {
        Ok(keys) => format!(
            "[{}]",
            keys.iter()
                .map(|k| format!("{k:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
    };

    let read = format!(
        "get={} scan={} file={:?}",
        match &got {
            Ok(Some(v)) => format!("Some({v})"),
            Ok(None) => "None".to_string(),
            Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
        },
        brief(&scan_render),
        on_disk
    );

    let scan_agrees = matches!(&scanned, Ok(keys) if keys.len() == 1 && keys[0] == path);

    let verdict = match (&got, scan_agrees) {
        (Ok(Some(7)), true) => "ok".to_string(),
        (Ok(Some(7)), false) => {
            "RESIDUE: the value reads back, and a scan of the level above does not name \
             the path it was written at"
                .to_string()
        }
        (Ok(Some(_)), _) => "SILENT ALTERATION: a different value".to_string(),
        (Ok(None), _) => "SILENT ALTERATION: nothing at the path".to_string(),
        (Err(_), _) => "SILENT ALTERATION: written Ok, unreadable".to_string(),
    };

    Row {
        probe: probe.to_string(),
        wrote: brief(&format!("{segment:?}")),
        write,
        read: brief(&read),
        verdict,
    }
}

#[test]
fn numbers() {
    let mut rows = Vec::new();
    let at = ["probe", "leaf"];

    probe!(rows, "u64::MAX", round_trip("u64::MAX", &at, &u64::MAX));
    probe!(rows, "i64::MIN", round_trip("i64::MIN", &at, &i64::MIN));
    probe!(rows, "u128::MAX", round_trip("u128::MAX", &at, &u128::MAX));
    probe!(rows, "i128::MIN", round_trip("i128::MIN", &at, &i128::MIN));
    probe!(
        rows,
        "u64 2^53+1",
        round_trip("u64 2^53+1", &at, &9007199254740993u64)
    );
    probe!(
        rows,
        "f64::MIN_POSITIVE",
        round_trip("f64::MIN_POSITIVE", &at, &f64::MIN_POSITIVE)
    );
    probe!(
        rows,
        "f64 subnormal",
        round_trip("f64 subnormal", &at, &5e-324f64)
    );
    probe!(rows, "f64::MAX", round_trip("f64::MAX", &at, &f64::MAX));
    probe!(
        rows,
        "f64 0.1+0.2",
        round_trip("f64 0.1+0.2", &at, &(0.1f64 + 0.2f64))
    );
    probe!(rows, "f64 -0.0", round_trip("f64 -0.0", &at, &-0.0f64));
    probe!(rows, "f64 1.0", round_trip("f64 1.0", &at, &1.0f64));
    probe!(
        rows,
        "f64::EPSILON",
        round_trip("f64::EPSILON", &at, &f64::EPSILON)
    );
    probe!(rows, "f32 0.1", round_trip("f32 0.1", &at, &0.1f32));
    probe!(rows, "f32 read as f64", f32_widened());

    table("Numbers", &rows);
}

/// A float written as `f32` and read back as `f64`, which is what a migration
/// that widens a field does.
fn f32_widened() -> Row {
    let file = TempPath::new("probe_json_widen");
    let path = StorePath::from_segments(["probe", "leaf"]);

    let write = {
        let store = open(&file).expect("a fresh store opens");
        let set = store.set(&path, &0.1f32);
        let _ = store.save_now();
        match set {
            Ok(()) => "Ok".to_string(),
            Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
        }
    };

    let store = open(&file).expect("the file reopens");
    let back = store.get::<f64>(&path);

    let read = format!("{back:?}");
    let verdict = match back {
        Ok(Some(v)) if v == 0.1f32 as f64 => "ok, the widening is exact".to_string(),
        Ok(Some(_)) => "the widened read is not the f32 that was written".to_string(),
        other => format!("unreadable as f64: {other:?}"),
    };

    Row {
        probe: "f32 read as f64".to_string(),
        wrote: brief(&format!("{:?}", 0.1f32)),
        write,
        read: brief(&read),
        verdict,
    }
}

#[test]
fn non_finite_floats() {
    let mut rows = Vec::new();
    let at = ["probe", "leaf"];

    probe!(rows, "f64::NAN", round_trip("f64::NAN", &at, &f64::NAN));
    probe!(
        rows,
        "f64::INFINITY",
        round_trip("f64::INFINITY", &at, &f64::INFINITY)
    );
    probe!(
        rows,
        "f64::NEG_INFINITY",
        round_trip("f64::NEG_INFINITY", &at, &f64::NEG_INFINITY)
    );
    probe!(rows, "f32::NAN", round_trip("f32::NAN", &at, &f32::NAN));
    probe!(
        rows,
        "NaN inside a vec of 4",
        round_trip(
            "NaN inside a vec of 4",
            &at,
            &vec![1.0f64, 2.0, f64::NAN, 4.0]
        )
    );
    probe!(
        rows,
        "NaN inside a struct",
        round_trip(
            "NaN inside a struct",
            &at,
            &Reading {
                name: "sensor".to_string(),
                value: f64::NAN,
            }
        )
    );
    probe!(
        rows,
        "NaN as Option<f64>",
        round_trip("NaN as Option<f64>", &at, &Some(f64::NAN))
    );

    table("Non-finite floats", &rows);
}

#[derive(Debug, Serialize, Deserialize)]
struct Reading {
    name: String,
    value: f64,
}

#[test]
fn strings() {
    let mut rows = Vec::new();
    let at = ["probe", "leaf"];

    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("one NUL", "\0".to_string()),
        ("NUL in the middle", "a\0b".to_string()),
        ("control chars", "\u{1}\u{7}\u{1b}\u{7f}".to_string()),
        ("lone CR", "\r".to_string()),
        ("lone LF", "\n".to_string()),
        ("CRLF around text", "a\r\nb".to_string()),
        ("emoji with ZWJ", "🙂👨‍👩‍👧‍👦".to_string()),
        ("combining mark", "e\u{301}".to_string()),
        ("precomposed é", "\u{e9}".to_string()),
        ("right-to-left marks", "\u{200f}a\u{202e}b".to_string()),
        ("BOM", "\u{feff}".to_string()),
        ("valid json in a string", "{\"a\": [1, 2]}".to_string()),
        ("a backslash", "\\".to_string()),
        ("a dot", ".".to_string()),
        ("escaped-looking", "a\\.b".to_string()),
        ("one megabyte", "x".repeat(1024 * 1024)),
        ("unpaired-looking escape", "\\u0041".to_string()),
        ("noncharacter", "\u{fffe}\u{ffff}".to_string()),
        ("private use", "\u{10ffff}".to_string()),
    ];

    for (name, value) in cases {
        probe!(rows, name, round_trip(name, &at, &value));
    }

    probe!(rows, "char NUL", round_trip("char NUL", &at, &'\0'));

    table("Strings", &rows);
}

#[test]
fn path_segments() {
    let mut rows = Vec::new();

    let cases: Vec<(&str, String)> = vec![
        ("empty segment", String::new()),
        ("a lone dot", ".".to_string()),
        ("two dots", "..".to_string()),
        ("a dot inside", "a.b".to_string()),
        ("a backslash", "\\".to_string()),
        ("a backslash inside", "a\\b".to_string()),
        ("a trailing backslash", "a\\".to_string()),
        ("backslash then dot", "\\.".to_string()),
        ("two backslashes", "\\\\".to_string()),
        ("a quote", "\"".to_string()),
        ("a newline", "\n".to_string()),
        ("a NUL", "\0".to_string()),
        ("a tab", "\t".to_string()),
        ("emoji", "🙂".to_string()),
        ("combining mark", "e\u{301}".to_string()),
        ("right-to-left mark", "\u{200f}".to_string()),
        ("ten thousand chars", "x".repeat(10_000)),
        ("__init", "__init".to_string()),
        ("meta", "meta".to_string()),
        ("schema", "schema".to_string()),
        ("log", "log".to_string()),
        ("__init.probe", "__init.probe".to_string()),
        ("a space", " ".to_string()),
        ("json punctuation", "{\"}:,[]".to_string()),
    ];

    for (name, segment) in cases {
        probe!(rows, name, path_probe(name, &segment));
    }

    probe!(rows, "two levels that join alike", joined_alike());
    probe!(rows, "a dot at the top level", dot_at_top());

    table("Path segments", &rows);
}

/// `["a.b"]` and `["a", "b"]` are different places whose joined forms differ
/// only by an escape. Both are written; both should still be there.
fn joined_alike() -> Row {
    let file = TempPath::new("probe_json_alike");
    let one = StorePath::from_segments(["a.b"]);
    let two = StorePath::from_segments(["a", "b"]);

    let write = {
        let store = open(&file).expect("a fresh store opens");
        let first = store.set(&one, &1u32);
        let second = store.set(&two, &2u32);
        let _ = store.save_now();
        format!(
            "{:?} then {:?}",
            first.map_err(|e| brief(&format!("{e:#}"))),
            second.map_err(|e| brief(&format!("{e:#}")))
        )
    };

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(&file).expect("the file reopens");
    let a = store.get::<u32>(&one);
    let b = store.get::<u32>(&two);

    let verdict = match (&a, &b) {
        (Ok(Some(1)), Ok(Some(2))) => "ok, the two are different places".to_string(),
        _ => "SILENT ALTERATION: one of the two lost its value".to_string(),
    };

    Row {
        probe: "[\"a.b\"] beside [\"a\",\"b\"]".to_string(),
        wrote: "1 at a.b as one level, 2 at a/b as two".to_string(),
        write: brief(&write),
        read: brief(&format!("{a:?} {b:?} file={}", text)),
        verdict,
    }
}

/// A level named `.` at the top of the document rather than under a prefix.
fn dot_at_top() -> Row {
    round_trip("a lone dot at the top level", &["."], &7u32)
}

#[derive(Debug, Serialize, Deserialize)]
struct UnitStruct;

#[derive(Debug, Serialize, Deserialize)]
struct EmptyStruct {}

#[derive(Debug, Serialize, Deserialize)]
struct Nested {
    inner: Inner,
}

#[derive(Debug, Serialize, Deserialize)]
struct Inner {
    leaf: u32,
}

#[derive(Debug, Serialize, Deserialize)]
enum Shape {
    Unit,
    Newtype(u32),
    Tuple(u32, u32),
    Struct { a: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Tagged {
    First { v: u32 },
    Second { v: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
enum TagShaped {
    #[serde(rename = "type")]
    Type,
    #[serde(rename = "content")]
    Content(u32),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Either {
    Count(u64),
    Ratio(f64),
}

#[derive(Debug, Serialize, Deserialize)]
struct Reserved {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "$schema")]
    schema: String,
}

/// A `Serialize` an application could write that names the same key twice.
#[derive(Debug)]
struct Twins {
    first: u32,
    second: u32,
}

impl Serialize for Twins {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("a", &self.first)?;
        map.serialize_entry("a", &self.second)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Twins {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, u32>::deserialize(d)?;
        Ok(Twins {
            first: map.get("a").copied().unwrap_or(0),
            second: map.get("a").copied().unwrap_or(0),
        })
    }
}

#[test]
fn structures() {
    let mut rows = Vec::new();
    let at = ["probe", "leaf"];

    probe!(
        rows,
        "unit struct",
        round_trip("unit struct", &at, &UnitStruct)
    );
    probe!(
        rows,
        "empty struct",
        round_trip("empty struct", &at, &EmptyStruct {})
    );
    probe!(rows, "unit ()", round_trip("unit ()", &at, &()));
    probe!(
        rows,
        "empty vec",
        round_trip("empty vec", &at, &Vec::<u32>::new())
    );
    probe!(
        rows,
        "empty map",
        round_trip("empty map", &at, &HashMap::<String, u32>::new())
    );
    probe!(
        rows,
        "Option::None",
        round_trip("Option::None", &at, &Option::<u32>::None)
    );
    probe!(
        rows,
        "Option<Option<u32>> Some(None)",
        round_trip(
            "Option<Option<u32>> Some(None)",
            &at,
            &Some(Option::<u32>::None)
        )
    );
    probe!(
        rows,
        "Option<Option<u32>> None",
        round_trip(
            "Option<Option<u32>> None",
            &at,
            &Option::<Option<u32>>::None
        )
    );
    probe!(
        rows,
        "vec of options",
        round_trip("vec of options", &at, &vec![Some(1u32), None, Some(3)])
    );
    probe!(
        rows,
        "enum unit variant",
        round_trip("enum unit variant", &at, &Shape::Unit)
    );
    probe!(
        rows,
        "enum newtype variant",
        round_trip("enum newtype variant", &at, &Shape::Newtype(5))
    );
    probe!(
        rows,
        "enum tuple variant",
        round_trip("enum tuple variant", &at, &Shape::Tuple(5, 6))
    );
    probe!(
        rows,
        "enum struct variant",
        round_trip("enum struct variant", &at, &Shape::Struct { a: 7 })
    );
    probe!(
        rows,
        "internally tagged enum",
        round_trip("internally tagged enum", &at, &Tagged::First { v: 1 })
    );
    probe!(
        rows,
        "variant renamed to `type`",
        round_trip("variant renamed to `type`", &at, &TagShaped::Type)
    );
    probe!(
        rows,
        "variant renamed to `content`",
        round_trip("variant renamed to `content`", &at, &TagShaped::Content(9))
    );
    probe!(
        rows,
        "untagged enum, float arm",
        round_trip("untagged enum, float arm", &at, &Either::Ratio(3.0))
    );
    probe!(
        rows,
        "untagged enum, int arm",
        round_trip("untagged enum, int arm", &at, &Either::Count(3))
    );
    probe!(
        rows,
        "fields named `type` and `$schema`",
        round_trip(
            "fields named `type` and `$schema`",
            &at,
            &Reserved {
                kind: "a".to_string(),
                schema: "b".to_string(),
            }
        )
    );
    probe!(
        rows,
        "a serializer naming one key twice",
        round_trip(
            "a serializer naming one key twice",
            &at,
            &Twins {
                first: 1,
                second: 2
            }
        )
    );
    probe!(
        rows,
        "map with an empty key",
        round_trip(
            "map with an empty key",
            &at,
            &HashMap::from([(String::new(), 1u32)])
        )
    );
    probe!(
        rows,
        "map with a dotted key",
        round_trip(
            "map with a dotted key",
            &at,
            &HashMap::from([("a.b".to_string(), 1u32)])
        )
    );
    probe!(
        rows,
        "map with a numeric key",
        round_trip(
            "map with a numeric key",
            &at,
            &HashMap::from([(7u32, 1u32)])
        )
    );
    probe!(
        rows,
        "map with a tuple key",
        round_trip(
            "map with a tuple key",
            &at,
            &BTreeMap::from([((1u8, 2u8), 1u32)])
        )
    );
    probe!(
        rows,
        "a nested struct",
        round_trip(
            "a nested struct",
            &at,
            &Nested {
                inner: Inner { leaf: 3 }
            }
        )
    );
    probe!(rows, "a tuple", round_trip("a tuple", &at, &(1u32, 2u32)));

    table("Structures", &rows);
}

#[test]
fn maps_and_collections() {
    let mut rows = Vec::new();

    probe!(rows, "a map of 2000 entries", many_entries(2000));
    probe!(rows, "map keys that need escaping", awkward_map_keys());
    probe!(rows, "cleared and refilled", cleared_and_refilled());
    probe!(rows, "a NaN in one map entry", nan_in_a_map());

    table("Collections", &rows);
}

fn many_entries(n: usize) -> Row {
    let file = TempPath::new("probe_json_many");

    {
        let store = open(&file).expect("a fresh store opens");
        let map = reactive_map_with_path_only::<String, u32>(
            &store,
            ["probe", "many"],
            HashMap::new(),
            Uuid::new_v4(),
        )
        .expect("the map was declared");
        for i in 0..n {
            map.insert(format!("k{i}"), &(i as u32)).expect("inserted");
        }
        store.save_now().expect("flushed");
    }

    let store = open(&file).expect("the file reopens");
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["probe", "many"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .expect("the map was declared again");

    let len = map.len();
    let missing: Vec<usize> = (0..n)
        .filter(|i| map.get(&format!("k{i}")) != Some(*i as u32))
        .collect();

    Row {
        probe: format!("a map of {n} entries"),
        wrote: format!("{n} entries"),
        write: "Ok".to_string(),
        read: format!("len={len}, wrong or missing: {}", missing.len()),
        verdict: if len == n && missing.is_empty() {
            "ok".to_string()
        } else {
            format!(
                "SILENT ALTERATION: {} entries came back changed or absent",
                missing.len()
            )
        },
    }
}

fn awkward_map_keys() -> Row {
    let file = TempPath::new("probe_json_keys");
    let keys = [
        "plain".to_string(),
        ".".to_string(),
        "..".to_string(),
        "a.b".to_string(),
        "\\".to_string(),
        "a\\b".to_string(),
        "a\\.b".to_string(),
        "\n".to_string(),
        "\0".to_string(),
        "🙂".to_string(),
        " ".to_string(),
        "\"".to_string(),
        String::new(),
    ];

    let refused = {
        let store = open(&file).expect("a fresh store opens");
        let map = reactive_map_with_path_only::<String, u32>(
            &store,
            ["probe", "keys"],
            HashMap::new(),
            Uuid::new_v4(),
        )
        .expect("the map was declared");

        let mut refused = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            if map.insert(key.clone(), &(i as u32)).is_err() {
                refused.push(key.clone());
            }
        }
        store.save_now().expect("flushed");
        refused
    };

    let store = open(&file).expect("the file reopens");
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["probe", "keys"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .expect("the map was declared again");

    let mut lost = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        if refused.contains(key) {
            continue;
        }
        if map.get(key) != Some(i as u32) {
            lost.push(format!("{key:?} -> {:?}", map.get(key)));
        }
    }

    let expected = keys.len() - refused.len();

    Row {
        probe: "map keys that need escaping".to_string(),
        wrote: brief(&format!("{keys:?}")),
        write: format!("refused: {refused:?}"),
        read: brief(&format!(
            "len={} of {expected}, wrong: {lost:?}, keys={:?}",
            map.len(),
            map.keys().collect::<Vec<_>>()
        )),
        verdict: if lost.is_empty() && map.len() == expected {
            "ok".to_string()
        } else {
            "SILENT ALTERATION: an accepted key did not come back".to_string()
        },
    }
}

fn cleared_and_refilled() -> Row {
    let file = TempPath::new("probe_json_clear");

    let after_clear = {
        let store = open(&file).expect("a fresh store opens");
        let map = reactive_map_with_path_only::<String, u32>(
            &store,
            ["probe", "cycle"],
            HashMap::new(),
            Uuid::new_v4(),
        )
        .expect("the map was declared");

        for i in 0..8u32 {
            map.insert(format!("k{i}"), &i).expect("inserted");
        }
        map.clear().expect("cleared");
        store.save_now().expect("flushed");
        let text = std::fs::read_to_string(file.path()).unwrap_or_default();

        for i in 0..3u32 {
            map.insert(format!("n{i}"), &i).expect("inserted again");
        }
        store.save_now().expect("flushed again");
        text
    };

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(&file).expect("the file reopens");
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["probe", "cycle"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .expect("the map was declared again");

    let leaves = leaf_paths(&text);
    let ghost = leaves
        .iter()
        .any(|leaf| leaf.first().map(String::as_str) == Some("probe") && leaf.len() < 3);

    Row {
        probe: "cleared and refilled".to_string(),
        wrote: "8 entries, clear, 3 entries".to_string(),
        write: "Ok".to_string(),
        read: brief(&format!(
            "len={} keys={:?} file after the clear={} leaves now={:?}",
            map.len(),
            map.keys().collect::<Vec<_>>(),
            brief(&after_clear),
            leaves
        )),
        verdict: match (map.len() == 3, ghost) {
            (true, false) => "ok".to_string(),
            (true, true) => "RESIDUE: an empty node is left where the map was".to_string(),
            (false, _) => {
                "SILENT ALTERATION: the refilled map is not what was put in it".to_string()
            }
        },
    }
}

fn nan_in_a_map() -> Row {
    let file = TempPath::new("probe_json_mapnan");

    let inserted = {
        let store = open(&file).expect("a fresh store opens");
        let map = reactive_map_with_path_only::<String, f64>(
            &store,
            ["probe", "nanmap"],
            HashMap::new(),
            Uuid::new_v4(),
        )
        .expect("the map was declared");
        map.insert("good".to_string(), &1.0).expect("inserted");
        let bad = map.insert("bad".to_string(), &f64::NAN);
        store.save_now().expect("flushed");
        format!("{:?}", bad.map_err(|e| brief(&format!("{e:#}"))))
    };

    let store = open(&file).expect("the file reopens");
    let map = reactive_map_with_path_only::<String, f64>(
        &store,
        ["probe", "nanmap"],
        HashMap::new(),
        Uuid::new_v4(),
    );

    let (read, verdict) = match map {
        Ok(map) => (
            format!(
                "len={} good={:?} bad={:?}",
                map.len(),
                map.get("good"),
                map.get("bad")
            ),
            if map.get("good") == Some(1.0) && map.get("bad").is_none() {
                "SILENT ALTERATION: the entry is gone and the map loads without it".to_string()
            } else {
                "see the reading".to_string()
            },
        ),
        Err(e) => (
            format!("Err: {}", brief(&format!("{e:#}"))),
            "OPAQUE FAILURE for the map: one bad entry and the whole map will not load".to_string(),
        ),
    };

    Row {
        probe: "a NaN in one map entry".to_string(),
        wrote: "1.0 at `good`, NaN at `bad`".to_string(),
        write: inserted,
        read: brief(&read),
        verdict,
    }
}

/// A value nested `depth` arrays deep, with a number at the bottom.
fn nested_value(depth: usize) -> serde_json::Value {
    let mut value = serde_json::Value::from(7u32);
    for _ in 0..depth {
        value = serde_json::Value::Array(vec![value]);
    }
    value
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Deep {
    Survived,
    WriteRefused,
    ReopenFailed,
    ReadFailed,
    ValueChanged,
}

/// Writes a value `value_depth` levels deep at a path `path_depth` levels down,
/// and says how far it got.
fn deep_outcome(path_depth: usize, value_depth: usize) -> Deep {
    let file = TempPath::new("probe_json_depth");
    let names: Vec<String> = (0..path_depth).map(|i| format!("s{i}")).collect();
    let path = StorePath::from_segments(&names);
    let value = nested_value(value_depth);

    {
        let store = match open(&file) {
            Ok(store) => store,
            Err(_) => return Deep::ReopenFailed,
        };
        if store.set(&path, &value).is_err() {
            return Deep::WriteRefused;
        }
        if store.save_now().is_err() {
            return Deep::WriteRefused;
        }
    }

    let store = match open(&file) {
        Ok(store) => store,
        Err(_) => return Deep::ReopenFailed,
    };

    match store.get::<serde_json::Value>(&path) {
        Ok(Some(back)) if back == value => Deep::Survived,
        Ok(Some(_)) => Deep::ValueChanged,
        Ok(None) => Deep::ValueChanged,
        Err(_) => Deep::ReadFailed,
    }
}

/// The largest `value_depth` that survives at this path depth, and what the
/// step past it does.
fn boundary(path_depth: usize, ceiling: usize) -> (usize, Deep) {
    if deep_outcome(path_depth, ceiling) == Deep::Survived {
        return (ceiling, Deep::Survived);
    }

    let mut good = 0usize;
    let mut bad = ceiling;
    while good + 1 < bad {
        let mid = (good + bad) / 2;
        if deep_outcome(path_depth, mid) == Deep::Survived {
            good = mid;
        } else {
            bad = mid;
        }
    }

    (good, deep_outcome(path_depth, bad))
}

#[test]
fn depth() {
    println!("\n## Depth\n");
    println!("A path of N levels and a value nested M arrays deep.\n");
    println!("| path levels | deepest value that survives | what the next one does |");
    println!("|---|---|---|");

    for path_depth in [1usize, 2, 3, 5, 10, 20] {
        let (deepest, past) = boundary(path_depth, 260);
        println!("| {path_depth} | {deepest} | {past:?} |");
    }

    println!("\nAnd the same question asked of the path alone, with a plain number at the end:\n");

    let mut good = 1usize;
    let mut bad = 300usize;
    if deep_outcome(bad, 0) == Deep::Survived {
        good = bad;
    } else {
        while good + 1 < bad {
            let mid = (good + bad) / 2;
            if deep_outcome(mid, 0) == Deep::Survived {
                good = mid;
            } else {
                bad = mid;
            }
        }
    }
    println!("deepest path that survives: {good} levels");
    println!(
        "one level further ({}): {:?}",
        good + 1,
        deep_outcome(good + 1, 0)
    );

    println!(
        "\nThe pair the two budgets share - value depth {} at path depth 2 and the same value at path depth 10:",
        120
    );
    println!("  at 2 levels: {:?}", deep_outcome(2, 120));
    println!("  at 10 levels: {:?}", deep_outcome(10, 120));
}

#[test]
fn ancestry_and_residue() {
    let mut rows = Vec::new();

    probe!(rows, "a struct field is a path", struct_field_is_a_path());
    probe!(
        rows,
        "a path overwrites a struct field",
        path_writes_into_a_struct()
    );
    probe!(
        rows,
        "an empty map over a branch",
        empty_map_over_a_branch()
    );
    probe!(rows, "delete leaves the parent", delete_leaves_the_parent());
    probe!(
        rows,
        "delete_prefix leaves the parent",
        delete_prefix_leaves_the_parent()
    );
    probe!(rows, "a scalar under a scalar", scalar_under_a_scalar());
    probe!(rows, "a map key no path can hold", unreachable_map_key());
    probe!(rows, "a write at the root", root_overwrite());
    probe!(rows, "delete takes the subtree", delete_takes_the_subtree());
    probe!(rows, "a field over a branch", field_over_a_branch());
    probe!(
        rows,
        "siblings that differ by an escape",
        sibling_escaping()
    );

    table("Beyond the list", &rows);
}

/// The root is a path like any other, and writing an object there is a write
/// the store accepts.
fn root_overwrite() -> Row {
    let file = TempPath::new("probe_json_root");

    let second = {
        let store = open(&file).expect("a fresh store opens");
        store.set(["kept"], &1u32).expect("written");
        let second = store.set(
            StorePath::root(),
            &HashMap::from([("new".to_string(), 2u32)]),
        );
        store.save_now().expect("flushed");
        format!("{:?}", second.map_err(|e| brief(&format!("{e:#}"))))
    };

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(&file).expect("the file reopens");
    let kept = store.get::<u32>(["kept"]);

    Row {
        probe: "a write at the root".to_string(),
        wrote: "1 at kept, then {\"new\": 2} at the root".to_string(),
        write: second,
        read: brief(&format!("kept={kept:?} file={text}")),
        verdict: match kept {
            Ok(None) => "SILENT ALTERATION: one write at the root replaced the whole document and \
                 returned Ok"
                .to_string(),
            Ok(Some(1)) => "ok, the root write did not take the rest with it".to_string(),
            other => format!("{other:?}"),
        },
    }
}

/// `delete` is the call for one value, and at a level holding none it takes
/// everything underneath.
fn delete_takes_the_subtree() -> Row {
    let file = TempPath::new("probe_json_del_subtree");

    let deleted = {
        let store = open(&file).expect("a fresh store opens");
        store.set(["probe", "cfg", "a"], &1u32).expect("written");
        store.set(["probe", "cfg", "b"], &2u32).expect("written");
        store.save_now().expect("flushed");
        let deleted = store.delete(&StorePath::from_segments(["probe", "cfg"]));
        store.save_now().expect("flushed again");
        format!("{:?}", deleted.map_err(|e| brief(&format!("{e:#}"))))
    };

    let store = open(&file).expect("the file reopens");
    let a = store.get::<u32>(["probe", "cfg", "a"]);
    let b = store.get::<u32>(["probe", "cfg", "b"]);

    Row {
        probe: "delete takes the subtree".to_string(),
        wrote: "1 and 2 under probe.cfg, then delete at probe.cfg".to_string(),
        write: deleted,
        read: brief(&format!("probe.cfg.a={a:?} probe.cfg.b={b:?}")),
        verdict: match (&a, &b) {
            (Ok(None), Ok(None)) => {
                "SILENT ALTERATION: a delete at a level holding no value of its own removed \
                 two values that were written"
                    .to_string()
            }
            _ => "ok, only the named path went".to_string(),
        },
    }
}

/// A typed field declared where the store already holds levels rather than a
/// value.
fn field_over_a_branch() -> Row {
    let file = TempPath::new("probe_json_field_branch");

    {
        let store = open(&file).expect("a fresh store opens");
        store.set(["probe", "cfg", "leaf"], &3u32).expect("written");
        store.save_now().expect("flushed");
    }

    let store = open(&file).expect("the file reopens");
    let field =
        amethystate::store::field_with_path::<u32>(&store, ["probe", "cfg"], 0, Uuid::new_v4());

    let (read, verdict) = match field {
        Ok(field) => {
            let set = field.set(5);
            (
                format!(
                    "get={} set={:?}",
                    field.get(),
                    set.map_err(|e| brief(&format!("{e:#}")))
                ),
                "the field was declared; see whether its writes land".to_string(),
            )
        }
        Err(e) => (
            format!("Err: {}", brief(&format!("{e:#}"))),
            "OPAQUE FAILURE for the application: a field cannot be declared at a path the \
             store holds levels under, so the struct owning it will not construct"
                .to_string(),
        ),
    };

    Row {
        probe: "a field over a branch".to_string(),
        wrote: "3 at probe.cfg.leaf, then a u32 field declared at probe.cfg".to_string(),
        write: "Ok".to_string(),
        read: brief(&read),
        verdict,
    }
}

/// Two levels whose joined forms differ only by an escape, with values under
/// both - the shape the kept proptest counterexample shrank to.
fn sibling_escaping() -> Row {
    let file = TempPath::new("probe_json_siblings");

    let under = StorePath::from_segments(["\\", "\\", "\\"]);
    let beside = StorePath::from_segments(["\\\\", "\\", "\\"]);
    let elsewhere = StorePath::from_segments(["a"]);
    let prefix = StorePath::from_segments(["\\"]);

    {
        let store = open(&file).expect("a fresh store opens");
        store.set(&under, &0u32).expect("written");
        store.set(&beside, &9000u32).expect("written");
        store.set(&elsewhere, &1u32).expect("written");
        store.save_now().expect("flushed");
    }

    let store = open(&file).expect("the file reopens");
    let a = store.get::<u32>(&under);
    let b = store.get::<u32>(&beside);
    let keys = store.scan_keys(&prefix);

    let listed_outside = match &keys {
        Ok(keys) => keys.iter().any(|k| !k.starts_with(&prefix)),
        Err(_) => false,
    };

    Row {
        probe: "siblings that differ by an escape".to_string(),
        wrote: format!("0 at {under}, 9000 at {beside}, 1 at {elsewhere}"),
        write: "Ok".to_string(),
        read: brief(&format!("under={a:?} beside={b:?} scan({prefix})={keys:?}")),
        verdict: match (&a, &b, listed_outside) {
            (Ok(Some(0)), Ok(Some(9000)), false) => {
                "ok by path; the scan of the prefix names only what is under it".to_string()
            }
            (Ok(Some(0)), Ok(Some(9000)), true) => {
                "RESIDUE: the scan of the prefix names a path that is not under it".to_string()
            }
            _ => "SILENT ALTERATION: one of the two did not read back".to_string(),
        },
    }
}

/// A struct written at one path puts a value at every path its fields spell.
fn struct_field_is_a_path() -> Row {
    let file = TempPath::new("probe_json_field_path");

    {
        let store = open(&file).expect("a fresh store opens");
        store
            .set(
                ["probe", "cfg"],
                &Nested {
                    inner: Inner { leaf: 3 },
                },
            )
            .expect("the struct was written");
        store.save_now().expect("flushed");
    }

    let store = open(&file).expect("the file reopens");
    let deeper = store.get::<u32>(["probe", "cfg", "inner", "leaf"]);
    let parent = store.get::<Inner>(["probe", "cfg", "inner"]);

    Row {
        probe: "a struct field is a path".to_string(),
        wrote: "Nested { inner: Inner { leaf: 3 } } at probe.cfg".to_string(),
        write: "Ok".to_string(),
        read: brief(&format!(
            "probe.cfg.inner.leaf={deeper:?} probe.cfg.inner={parent:?}"
        )),
        verdict: match deeper {
            Ok(Some(3)) => {
                "RESIDUE: paths nobody wrote read back as values, because the value's own \
                 shape became levels"
                    .to_string()
            }
            other => format!("nothing readable underneath: {other:?}"),
        },
    }
}

/// And the other way: writing a path underneath a struct edits the struct.
fn path_writes_into_a_struct() -> Row {
    let file = TempPath::new("probe_json_into_struct");

    let second = {
        let store = open(&file).expect("a fresh store opens");
        store
            .set(["probe", "cfg"], &Inner { leaf: 3 })
            .expect("the struct was written");
        let second = store.set(["probe", "cfg", "leaf"], &99u32);
        store.save_now().expect("flushed");
        format!("{:?}", second.map_err(|e| brief(&format!("{e:#}"))))
    };

    let store = open(&file).expect("the file reopens");
    let back = store.get::<Inner>(["probe", "cfg"]);

    Row {
        probe: "a path overwrites a struct field".to_string(),
        wrote: "Inner { leaf: 3 } at probe.cfg, then 99 at probe.cfg.leaf".to_string(),
        write: second,
        read: brief(&format!("probe.cfg={back:?}")),
        verdict: match back {
            Ok(Some(Inner { leaf: 99 })) => {
                "SILENT ALTERATION: the value at probe.cfg changed without anyone writing to it"
                    .to_string()
            }
            other => format!("{other:?}"),
        },
    }
}

/// An empty map is a value the document cannot tell from a level, so writing
/// one over a level takes the level with it.
fn empty_map_over_a_branch() -> Row {
    let file = TempPath::new("probe_json_empty_over");

    let second = {
        let store = open(&file).expect("a fresh store opens");
        store.set(["probe", "cfg", "leaf"], &3u32).expect("written");
        let second = store.set(["probe", "cfg"], &HashMap::<String, u32>::new());
        store.save_now().expect("flushed");
        format!("{:?}", second.map_err(|e| brief(&format!("{e:#}"))))
    };

    let store = open(&file).expect("the file reopens");
    let leaf = store.get::<u32>(["probe", "cfg", "leaf"]);

    Row {
        probe: "an empty map over a branch".to_string(),
        wrote: "3 at probe.cfg.leaf, then {} at probe.cfg".to_string(),
        write: second,
        read: brief(&format!("probe.cfg.leaf={leaf:?}")),
        verdict: match leaf {
            Ok(None) => "SILENT ALTERATION: a write at probe.cfg deleted what was under it and \
                 returned Ok"
                .to_string(),
            Ok(Some(3)) => "ok, the write was refused or did not reach it".to_string(),
            other => format!("{other:?}"),
        },
    }
}

/// After the only value under a level goes, the level stays.
fn delete_leaves_the_parent() -> Row {
    let file = TempPath::new("probe_json_del_parent");

    {
        let store = open(&file).expect("a fresh store opens");
        store.set(["probe", "cfg", "leaf"], &3u32).expect("written");
        store.save_now().expect("flushed");
        store
            .delete(&StorePath::from_segments(["probe", "cfg", "leaf"]))
            .expect("deleted");
        store.save_now().expect("flushed again");
    }

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(&file).expect("the file reopens");
    let parent = store.get::<HashMap<String, u32>>(["probe", "cfg"]);
    let keys = store.scan_keys(&StorePath::segment("probe"));

    Row {
        probe: "delete leaves the parent".to_string(),
        wrote: "3 at probe.cfg.leaf, then deleted".to_string(),
        write: "Ok".to_string(),
        read: brief(&format!(
            "probe.cfg={parent:?} scan(probe)={keys:?} file={text}"
        )),
        verdict: match parent {
            Ok(Some(map)) if map.is_empty() => {
                "RESIDUE: an empty node stands where nobody wrote a value".to_string()
            }
            Ok(None) => "ok, nothing is left behind".to_string(),
            other => format!("{other:?}"),
        },
    }
}

fn delete_prefix_leaves_the_parent() -> Row {
    let file = TempPath::new("probe_json_del_prefix");

    {
        let store = open(&file).expect("a fresh store opens");
        store.set(["probe", "cfg", "a"], &1u32).expect("written");
        store.set(["probe", "cfg", "b"], &2u32).expect("written");
        store.save_now().expect("flushed");
        store
            .delete_prefix_with_source(&StorePath::from_segments(["probe", "cfg"]), None)
            .expect("prefix deleted");
        store.save_now().expect("flushed again");
    }

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(&file).expect("the file reopens");
    let parent = store.get::<HashMap<String, u32>>(["probe", "cfg"]);
    let root = store.get::<HashMap<String, serde_json::Value>>(["probe"]);

    Row {
        probe: "delete_prefix leaves the parent".to_string(),
        wrote: "two values under probe.cfg, then the prefix deleted".to_string(),
        write: "Ok".to_string(),
        read: brief(&format!("probe.cfg={parent:?} probe={root:?} file={text}")),
        verdict: match (&parent, &root) {
            (Ok(None), Ok(Some(map))) if map.is_empty() => {
                "RESIDUE: probe is left as an empty node".to_string()
            }
            (Ok(None), Ok(None)) => "ok, nothing is left behind".to_string(),
            other => format!("{other:?}"),
        },
    }
}

fn scalar_under_a_scalar() -> Row {
    let file = TempPath::new("probe_json_scalar_under");

    let second = {
        let store = open(&file).expect("a fresh store opens");
        store.set(["probe", "cfg"], &3u32).expect("written");
        let second = store.set(["probe", "cfg", "leaf"], &4u32);
        store.save_now().expect("flushed");
        format!("{:?}", second.map_err(|e| brief(&format!("{e:#}"))))
    };

    let store = open(&file).expect("the file reopens");
    let outer = store.get::<u32>(["probe", "cfg"]);
    let inner = store.get::<u32>(["probe", "cfg", "leaf"]);

    Row {
        probe: "a scalar under a scalar".to_string(),
        wrote: "3 at probe.cfg, then 4 at probe.cfg.leaf".to_string(),
        write: second,
        read: brief(&format!("probe.cfg={outer:?} probe.cfg.leaf={inner:?}")),
        verdict: match (&outer, &inner) {
            (Ok(Some(3)), Ok(None)) => "ok, refused and nothing moved".to_string(),
            other => format!("{other:?}"),
        },
    }
}

/// A map value whose key cannot be a level: the key is in the file, `get` on
/// the whole value finds it, and every path-shaped read passes it by.
fn unreachable_map_key() -> Row {
    let file = TempPath::new("probe_json_bad_key");

    {
        let store = open(&file).expect("a fresh store opens");
        store
            .set(
                ["probe", "cfg"],
                &HashMap::from([(String::new(), 1u32), ("ok".to_string(), 2u32)]),
            )
            .expect("written");
        store.save_now().expect("flushed");
    }

    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(&file).expect("the file reopens");
    let whole = store.get::<HashMap<String, u32>>(["probe", "cfg"]);
    let keys = store.scan_keys(&StorePath::from_segments(["probe", "cfg"]));
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["probe", "cfg"],
        HashMap::new(),
        Uuid::new_v4(),
    );

    let map_len = match &map {
        Ok(map) => format!("{}", map.len()),
        Err(e) => format!("Err: {}", brief(&format!("{e:#}"))),
    };

    Row {
        probe: "a map key no path can hold".to_string(),
        wrote: "{\"\": 1, \"ok\": 2} at probe.cfg".to_string(),
        write: "Ok".to_string(),
        read: brief(&format!(
            "whole={whole:?} scan={keys:?} ReactiveMap len={map_len} file={text}"
        )),
        verdict: "see the reading: the empty key is in the file and no path reaches it".to_string(),
    }
}
