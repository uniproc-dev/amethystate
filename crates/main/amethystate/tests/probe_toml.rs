#![cfg(feature = "toml")]

//! What the toml engine takes on a write and does not hand back on a read.
//!
//! Every probe here writes through a store, flushes, drops it, opens the file
//! again and reads the same path back. Nothing is inferred from the source:
//! the row a probe prints is what the run produced.
//!
//! Run with `cargo test -p amethystate --features toml --test probe_toml --
//! --nocapture` to see the tables.
//!
//! ## What toml cannot represent
//!
//! **There is no null.** A key that holds nothing is a key that is not
//! written, so `set(None)` and `delete` leave the same document. The other
//! engines keep those two apart - json writes `null`, ron writes `None`.
//!
//! **An empty table is a value.** A level with nothing under it is written and
//! read back, which is not the same as a level that is absent. So toml can say
//! "this level exists and is empty" while it cannot say "this key exists and
//! holds nothing".

use amethystate::Store;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Debug};
use std::panic::AssertUnwindSafe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Written, read back, equal.
    Faithful,
    /// The write said no, which is the honest answer.
    WriteRefused,
    /// The file or the store will not open afterwards.
    OpaqueFailure,
    /// It comes back different, or as an error, with nothing said at write.
    SilentAlteration,
    /// It comes back as nothing at all.
    Absent,
    /// Something unwound.
    Panic,
    /// A scripted probe whose answer is the read column itself.
    Observed,
}

impl Verdict {
    fn tag(self) -> &'static str {
        match self {
            Verdict::Faithful => "faithful",
            Verdict::WriteRefused => "write refused",
            Verdict::OpaqueFailure => "OPAQUE FAILURE",
            Verdict::SilentAlteration => "SILENT ALTERATION",
            Verdict::Absent => "ABSENT ON READ",
            Verdict::Panic => "PANIC",
            Verdict::Observed => "observed",
        }
    }
}

struct Row {
    label: String,
    wrote: String,
    write: String,
    read: String,
    file: String,
    keys: Vec<String>,
    verdict: Verdict,
}

impl Row {
    fn blank(label: &str) -> Self {
        Row {
            label: label.to_string(),
            wrote: String::new(),
            write: "-".into(),
            read: "-".into(),
            file: String::new(),
            keys: Vec::new(),
            verdict: Verdict::Faithful,
        }
    }
}

fn brief(text: &str, keep: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| match c {
            '\n' => '\u{21b5}',
            '\r' => '\u{240d}',
            '\t' => '\u{2192}',
            '\0' => '\u{2400}',
            other => other,
        })
        .collect();
    if flat.chars().count() <= keep {
        flat
    } else {
        let head: String = flat.chars().take(keep).collect();
        format!("{head}...[{} chars]", flat.chars().count())
    }
}

fn why<T: Debug>(err: &T) -> String {
    let text = format!("{err:?}");
    let head: Vec<String> = text
        .lines()
        .map(|l| {
            l.trim_start_matches([
                '\u{2502}', '\u{251c}', '\u{2570}', '\u{2574}', '\u{2500}', '\u{25b6}', ' ',
            ])
            .trim()
            .to_string()
        })
        .filter(|l| !l.is_empty() && !l.starts_with("at ") && !l.starts_with("backtrace"))
        .take_while(|l| !l.starts_with('\u{2501}'))
        .take(4)
        .collect();
    brief(&head.join(" / "), 220)
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return brief(s, 120);
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return brief(s, 120);
    }
    "<non-string panic>".into()
}

/// A label is prose; a file name on Windows is not. Everything outside
/// `[a-z0-9]` goes, or the temp file cannot be created and every probe whose
/// label held a colon reports a failure that is the harness's own.
fn temp_for(label: &str) -> TempPath {
    let stem: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(48)
        .collect();
    TempPath::new(&stem)
}

fn open(path: &std::path::Path) -> amethystate::StorageResult<Store> {
    StoreBuilder::new(path).backend(Backend::Toml).build()
}

fn root_keys(store: &Store) -> Vec<String> {
    store
        .scan_keys(StorePath::root())
        .map(|keys| keys.iter().map(|k| k.as_str().to_string()).collect())
        .unwrap_or_else(|e| vec![format!("<scan failed: {}>", why(&e))])
}

/// Every path a scan can reach, level by level - `scan_keys` answers with one
/// level of children, so what a caller can address is only visible by walking.
fn every_key(store: &Store) -> Vec<String> {
    fn walk(store: &Store, at: amethystate_core::path::StorePath, out: &mut Vec<String>) {
        if out.len() > 400 || at.len() > 40 {
            out.push(format!("<walk gave up at {at}>"));
            return;
        }
        match store.scan_keys(&at) {
            Ok(children) if children.is_empty() => {
                if !at.is_root() {
                    out.push(at.as_str().to_string());
                }
            }
            Ok(children) => {
                for child in children {
                    if child.as_str() == at.as_str() {
                        out.push(at.as_str().to_string());
                        continue;
                    }
                    walk(store, child, out);
                }
            }
            Err(e) => out.push(format!("<scan failed at {at}: {}>", why(&e))),
        }
    }

    let mut out = Vec::new();
    walk(
        store,
        amethystate_core::path::StorePath::from_segments(Vec::<String>::new()),
        &mut out,
    );
    out
}

/// Writes `value` at `segments`, flushes, reopens the file, reads it back.
fn probe<T>(label: &str, segments: &[&str], value: T) -> Row
where
    T: Serialize + DeserializeOwned + Debug + PartialEq,
{
    let wrote = brief(&format!("{value:?}"), 70);
    let segs: Vec<String> = segments.iter().map(|s| (*s).to_string()).collect();

    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| run(label, &segs, &value)));

    match caught {
        Ok(mut row) => {
            row.wrote = wrote;
            row
        }
        Err(payload) => {
            let mut row = Row::blank(label);
            row.wrote = wrote;
            row.read = panic_text(&*payload);
            row.verdict = Verdict::Panic;
            row
        }
    }
}

fn run<T>(label: &str, segs: &[String], value: &T) -> Row
where
    T: Serialize + DeserializeOwned + Debug + PartialEq,
{
    let tmp = temp_for(label);
    let mut row = Row::blank(label);

    {
        let store = match open(tmp.path()) {
            Ok(store) => store,
            Err(e) => {
                row.write = format!("open failed: {}", why(&e));
                row.verdict = Verdict::OpaqueFailure;
                return row;
            }
        };
        match store.set(segs.to_vec(), value) {
            Ok(()) => row.write = "Ok".into(),
            Err(e) => {
                row.write = format!("Err: {}", why(&e));
                row.verdict = Verdict::WriteRefused;
                return row;
            }
        }
        if let Err(e) = store.save_now() {
            row.write = format!("Ok, flush Err: {}", why(&e));
            row.verdict = Verdict::WriteRefused;
            return row;
        }
    }

    row.file = std::fs::read_to_string(tmp.path()).unwrap_or_else(|e| format!("<unreadable: {e}>"));

    let store = match open(tmp.path()) {
        Ok(store) => store,
        Err(e) => {
            row.read = format!("reopen Err: {}", why(&e));
            row.verdict = Verdict::OpaqueFailure;
            return row;
        }
    };

    match store.get::<T>(segs.to_vec()) {
        Ok(Some(back)) => {
            row.read = brief(&format!("{back:?}"), 70);
            row.verdict = if back == *value {
                Verdict::Faithful
            } else {
                Verdict::SilentAlteration
            };
        }
        Ok(None) => {
            row.read = "nothing".into();
            row.verdict = Verdict::Absent;
        }
        Err(e) => {
            row.read = format!("Err: {}", why(&e));
            row.verdict = Verdict::SilentAlteration;
        }
    }

    row.keys = root_keys(&store);
    row
}

fn table(title: &str, rows: &[Row]) {
    println!("\n### {title}");
    for row in rows {
        println!(
            "  {:<34} | wrote {:<40} | write {:<28} | read {:<44} | {}",
            row.label,
            row.wrote,
            row.write,
            row.read,
            row.verdict.tag()
        );
        if !row.file.is_empty() {
            println!("      file: {}", brief(&row.file, 160));
        }
        if !row.keys.is_empty() {
            println!("      keys: {:?}", row.keys);
        }
    }
}

#[test]
fn options() {
    let rows = vec![
        probe("Some(u32) at a leaf", &["opt", "some"], Some(7u32)),
        probe("None at a leaf", &["opt", "none"], Option::<u32>::None),
        probe(
            "None as the only field of a table",
            &["lonely", "only"],
            Option::<u32>::None,
        ),
        probe("Some(Some)", &["opt", "ss"], Some(Some(7u32))),
        probe("Some(None)", &["opt", "sn"], Some(Option::<u32>::None)),
        probe(
            "None nested twice",
            &["opt", "nn"],
            Option::<Option<u32>>::None,
        ),
        probe(
            "struct with a None field",
            &["opt", "struct"],
            WithOption {
                kept: 1,
                gone: None,
            },
        ),
        probe(
            "struct whose only field is None",
            &["opt", "onlynone"],
            OnlyOption { gone: None },
        ),
        probe("Some(\"\")", &["opt", "empty"], Some(String::new())),
        probe(
            "Vec of Options",
            &["opt", "vec"],
            vec![Some(1u32), None, Some(3)],
        ),
    ];
    table("1. Option", &rows);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct WithOption {
    kept: u32,
    gone: Option<u32>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct OnlyOption {
    gone: Option<u32>,
}

#[test]
fn non_finite_floats() {
    let mut rows = vec![
        probe("f64::INFINITY", &["f", "inf"], f64::INFINITY),
        probe("f64::NEG_INFINITY", &["f", "neginf"], f64::NEG_INFINITY),
        probe("f32::INFINITY", &["f", "inf32"], f32::INFINITY),
    ];

    let nan = run_raw(
        "probe_nan",
        &["f", "nan"],
        &f64::NAN,
        |store, segs| match store.get::<f64>(segs.to_vec()) {
            Ok(Some(v)) => (
                format!("{v} (is_nan {}, sign {})", v.is_nan(), v.is_sign_negative()),
                if v.is_nan() && !v.is_sign_negative() {
                    Verdict::Faithful
                } else {
                    Verdict::SilentAlteration
                },
            ),
            Ok(None) => ("nothing".into(), Verdict::Absent),
            Err(e) => (format!("Err: {}", why(&e)), Verdict::SilentAlteration),
        },
    );
    rows.push(nan);

    let neg_nan = run_raw(
        "probe_negnan",
        &["f", "negnan"],
        &-f64::NAN,
        |store, segs| match store.get::<f64>(segs.to_vec()) {
            Ok(Some(v)) => (
                format!("{v} (is_nan {}, sign {})", v.is_nan(), v.is_sign_negative()),
                if v.is_nan() && v.is_sign_negative() {
                    Verdict::Faithful
                } else {
                    Verdict::SilentAlteration
                },
            ),
            Ok(None) => ("nothing".into(), Verdict::Absent),
            Err(e) => (format!("Err: {}", why(&e)), Verdict::SilentAlteration),
        },
    );
    rows.push(neg_nan);

    table("2. non-finite floats", &rows);
}

/// A probe whose comparison is not `PartialEq` - a NaN never equals itself, so
/// the check has to be spelled out.
fn run_raw<T, F>(label: &str, segments: &[&str], value: &T, read: F) -> Row
where
    T: Serialize + Debug,
    F: FnOnce(&Store, &[String]) -> (String, Verdict),
{
    let segs: Vec<String> = segments.iter().map(|s| (*s).to_string()).collect();
    let tmp = temp_for(label);
    let mut row = Row::blank(label);
    row.wrote = brief(&format!("{value:?}"), 70);

    {
        let store = match open(tmp.path()) {
            Ok(store) => store,
            Err(e) => {
                row.write = format!("open failed: {}", why(&e));
                row.verdict = Verdict::OpaqueFailure;
                return row;
            }
        };
        match store.set(segs.to_vec(), value) {
            Ok(()) => row.write = "Ok".into(),
            Err(e) => {
                row.write = format!("Err: {}", why(&e));
                row.verdict = Verdict::WriteRefused;
                return row;
            }
        }
        if let Err(e) = store.save_now() {
            row.write = format!("Ok, flush Err: {}", why(&e));
            row.verdict = Verdict::WriteRefused;
            return row;
        }
    }

    row.file = std::fs::read_to_string(tmp.path()).unwrap_or_else(|e| format!("<unreadable: {e}>"));

    let store = match open(tmp.path()) {
        Ok(store) => store,
        Err(e) => {
            row.read = format!("reopen Err: {}", why(&e));
            row.verdict = Verdict::OpaqueFailure;
            return row;
        }
    };

    let (read, verdict) = read(&store, &segs);
    row.read = read;
    row.verdict = verdict;
    row.keys = root_keys(&store);
    row
}

#[test]
fn numbers() {
    let rows = vec![
        probe("u64::MAX", &["n", "u64max"], u64::MAX),
        probe("i64::MAX", &["n", "i64max"], i64::MAX),
        probe("i64::MIN", &["n", "i64min"], i64::MIN),
        probe(
            "u64 just past i64::MAX",
            &["n", "past"],
            i64::MAX as u64 + 1,
        ),
        probe("u128::MAX", &["n", "u128max"], u128::MAX),
        probe("i128::MIN", &["n", "i128min"], i128::MIN),
        probe("usize::MAX", &["n", "usizemax"], usize::MAX),
        probe("-0.0f64", &["n", "negzero"], -0.0f64),
        probe("f64::MIN_POSITIVE", &["n", "tiny"], f64::MIN_POSITIVE),
        probe("f64::MAX", &["n", "fmax"], f64::MAX),
        probe("f64 many digits", &["n", "digits"], 0.1f64 + 0.2f64),
        probe("f64 whole number", &["n", "whole"], 3.0f64),
        probe("f32 one third", &["n", "third32"], 1.0f32 / 3.0),
        probe("u8::MAX", &["n", "u8"], u8::MAX),
        probe("char", &["n", "char"], 'q'),
        probe("bool", &["n", "bool"], true),
    ];
    table("3. numbers", &rows);
}

#[test]
fn strings() {
    let rows = vec![
        probe("empty", &["s", "empty"], String::new()),
        probe("embedded NUL", &["s", "nul"], "a\0b".to_string()),
        probe(
            "control chars",
            &["s", "ctrl"],
            "a\u{1}\u{7}\u{1f}b".to_string(),
        ),
        probe("CRLF", &["s", "crlf"], "a\r\nb".to_string()),
        probe("lone CR", &["s", "cr"], "a\rb".to_string()),
        probe("backslash", &["s", "bs"], "a\\b".to_string()),
        probe("triple quote", &["s", "triple"], "a\"\"\"b".to_string()),
        probe(
            "looks like a datetime",
            &["s", "dt"],
            "1979-05-27T07:32:00Z".to_string(),
        ),
        probe(
            "looks like a local date",
            &["s", "date"],
            "1979-05-27".to_string(),
        ),
        probe("looks like a number", &["s", "num"], "1234".to_string()),
        probe("looks like true", &["s", "bool"], "true".to_string()),
        probe(
            "unicode",
            &["s", "uni"],
            "\u{1f600} \u{4e2d}\u{6587} \u{0301}".to_string(),
        ),
        probe("very long", &["s", "long"], "x".repeat(200_000)),
        probe("bare newline", &["s", "nl"], "a\nb".to_string()),
        probe("trailing whitespace", &["s", "ws"], "a   ".to_string()),
        probe("delete char", &["s", "del"], "a\u{7f}b".to_string()),
        probe("BOM", &["s", "bom"], "\u{feff}a".to_string()),
        probe(
            "lone surrogate-ish escape",
            &["s", "esc"],
            "a\\u0041b".to_string(),
        ),
    ];
    table("4. strings", &rows);
}

#[test]
fn keys() {
    let mut rows = vec![
        probe("plain", &["k", "plain"], 42u32),
        probe("segment holding a dot", &["k", "a.b"], 42u32),
        probe("segment holding a backslash", &["k", "a\\b"], 42u32),
        probe("segment holding a quote", &["k", "a\"b"], 42u32),
        probe("segment holding a single quote", &["k", "a'b"], 42u32),
        probe("segment holding a newline", &["k", "a\nb"], 42u32),
        probe("segment holding a CR", &["k", "a\rb"], 42u32),
        probe("segment holding a NUL", &["k", "a\0b"], 42u32),
        probe("segment holding spaces", &["k", "a b"], 42u32),
        probe("segment holding an equals", &["k", "a=b"], 42u32),
        probe("segment holding brackets", &["k", "[a]"], 42u32),
        probe("segment holding a hash", &["k", "a#b"], 42u32),
        probe("unicode segment", &["k", "\u{6e2c}\u{8a66}"], 42u32),
        probe("emoji segment", &["k", "\u{1f600}"], 42u32),
        probe("segment that is only spaces", &["k", "   "], 42u32),
        probe("very long segment", &["k", &"s".repeat(20_000)], 42u32),
        probe("segment named like meta", &["__init", "ns"], 42u32),
        probe(
            "segment named amethystate",
            &["amethystate", "watch_interval_ms"],
            42u32,
        ),
        probe("segment that is a number", &["k", "1979"], 42u32),
        probe("segment with a leading dot", &["k", ".lead"], 42u32),
        probe("segment that is a single dot", &["k", "."], 42u32),
        probe("segment that is two dots", &["k", ".."], 42u32),
        probe("segment holding a tab", &["k", "a\tb"], 42u32),
    ];

    rows.push(empty_segment_row());
    table("5. path segments", &rows);
}

fn empty_segment_row() -> Row {
    let mut row = Row::blank("empty segment");
    row.wrote = "42".into();
    let tmp = TempPath::new("probe_empty_seg");
    let store = match open(tmp.path()) {
        Ok(store) => store,
        Err(e) => {
            row.write = format!("open failed: {}", why(&e));
            row.verdict = Verdict::OpaqueFailure;
            return row;
        }
    };
    match store.set(vec!["k".to_string(), String::new()], &42u32) {
        Ok(()) => {
            row.write = "Ok".into();
            row.read = format!(
                "{:?}",
                store.get::<u32>(vec!["k".to_string(), String::new()])
            );
            row.verdict = Verdict::SilentAlteration;
        }
        Err(e) => {
            row.write = format!("Err: {}", why(&e));
            row.verdict = Verdict::WriteRefused;
        }
    }
    row
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Empty {}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Unit;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct NewType(u32);

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Tuple(u32, String);

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Choice {
    Bare,
    Payload(u32),
    Pair(u32, u32),
    Named { a: u32 },
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Server {
    name: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Mixed {
    scalar: u32,
    table: Inner,
    after: u32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Inner {
    x: u32,
}

#[test]
fn structures() {
    let rows = vec![
        probe("empty struct", &["st", "emptystruct"], Empty {}),
        probe("unit struct", &["st", "unit"], Unit),
        probe("unit value ()", &["st", "unitval"], ()),
        probe(
            "empty map",
            &["st", "emptymap"],
            BTreeMap::<String, u32>::new(),
        ),
        probe("empty vec", &["st", "emptyvec"], Vec::<u32>::new()),
        probe("vec of scalars", &["st", "vec"], vec![1u32, 2, 3]),
        probe(
            "vec of tables",
            &["st", "aot"],
            vec![Server { name: "a".into() }, Server { name: "b".into() }],
        ),
        probe(
            "empty vec of tables",
            &["st", "aotempty"],
            Vec::<Server>::new(),
        ),
        probe("unit enum variant", &["st", "bare"], Choice::Bare),
        probe(
            "newtype enum variant",
            &["st", "payload"],
            Choice::Payload(3),
        ),
        probe("tuple enum variant", &["st", "pair"], Choice::Pair(1, 2)),
        probe(
            "struct enum variant",
            &["st", "named"],
            Choice::Named { a: 5 },
        ),
        probe("newtype struct", &["st", "newtype"], NewType(9)),
        probe("tuple struct", &["st", "tuplestruct"], Tuple(9, "x".into())),
        probe("tuple", &["st", "tuple"], (1u32, "x".to_string())),
        probe(
            "scalar, then table, then scalar",
            &["st", "mixed"],
            Mixed {
                scalar: 1,
                table: Inner { x: 2 },
                after: 3,
            },
        ),
        probe(
            "map with an awkward key",
            &["st", "mapkey"],
            BTreeMap::from([("a.b".to_string(), 1u32), ("c\nd".to_string(), 2u32)]),
        ),
        probe(
            "map with an empty key",
            &["st", "mapempty"],
            BTreeMap::from([(String::new(), 1u32)]),
        ),
        probe(
            "nested vec",
            &["st", "nestvec"],
            vec![vec![1u32], vec![2, 3]],
        ),
        probe("bytes", &["st", "bytes"], vec![0u8, 1, 255]),
    ];
    table("6. structures", &rows);
}

/// A store built once, written to several times, then read back - which is
/// where table placement lives, since a single write cannot produce it.
fn scripted(
    label: &str,
    write: impl FnOnce(&Store) -> String,
    read: impl FnOnce(&Store) -> String,
) -> Row {
    let mut row = Row::blank(label);
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let tmp = temp_for(label);
        let mut row = Row::blank(label);
        {
            let store = match open(tmp.path()) {
                Ok(store) => store,
                Err(e) => {
                    row.write = format!("open failed: {}", why(&e));
                    row.verdict = Verdict::OpaqueFailure;
                    return row;
                }
            };
            row.wrote = write(&store);
            if let Err(e) = store.save_now() {
                row.write = format!("flush Err: {}", why(&e));
                row.verdict = Verdict::WriteRefused;
                return row;
            }
            row.write = "Ok".into();
        }
        row.file =
            std::fs::read_to_string(tmp.path()).unwrap_or_else(|e| format!("<unreadable: {e}>"));

        let store = match open(tmp.path()) {
            Ok(store) => store,
            Err(e) => {
                row.read = format!("reopen Err: {}", why(&e));
                row.verdict = Verdict::OpaqueFailure;
                return row;
            }
        };
        row.read = read(&store);
        row.keys = root_keys(&store);
        row.verdict = if row.read.starts_with("OK") {
            Verdict::Faithful
        } else if row.read.contains("wanted") || row.read.contains("Err ") {
            Verdict::SilentAlteration
        } else {
            Verdict::Observed
        };
        row
    }));

    match caught {
        Ok(row) => row,
        Err(payload) => {
            row.read = panic_text(&*payload);
            row.verdict = Verdict::Panic;
            row
        }
    }
}

fn expect<T: Debug + PartialEq>(
    label: &str,
    got: amethystate::StorageResult<Option<T>>,
    want: Option<T>,
) -> String {
    match got {
        Ok(v) if v == want => format!("OK {label}"),
        Ok(v) => format!(
            "{label}: got {:?}, wanted {:?}",
            brief(&format!("{v:?}"), 40),
            want
        ),
        Err(e) => format!("{label}: Err {}", why(&e)),
    }
}

#[test]
fn ordering_and_placement() {
    let rows = vec![
        scripted(
            "table written, then a sibling scalar",
            |store| {
                store.set(["cfg", "inner", "x"], &1u32).unwrap();
                store.set(["cfg", "flat"], &2u32).unwrap();
                "cfg.inner.x=1 then cfg.flat=2".into()
            },
            |store| {
                let a = expect("inner", store.get::<u32>(["cfg", "inner", "x"]), Some(1u32));
                let b = expect("flat", store.get::<u32>(["cfg", "flat"]), Some(2u32));
                join(&[a, b])
            },
        ),
        scripted(
            "scalar written, then a sibling table",
            |store| {
                store.set(["cfg", "flat"], &2u32).unwrap();
                store.set(["cfg", "inner", "x"], &1u32).unwrap();
                "cfg.flat=2 then cfg.inner.x=1".into()
            },
            |store| {
                let a = expect("inner", store.get::<u32>(["cfg", "inner", "x"]), Some(1u32));
                let b = expect("flat", store.get::<u32>(["cfg", "flat"]), Some(2u32));
                join(&[a, b])
            },
        ),
        scripted(
            "scalar replaced by a table",
            |store| {
                store.set(["cfg", "node"], &2u32).unwrap();
                let second = store.set(["cfg", "node", "x"], &1u32);
                format!("cfg.node=2 then cfg.node.x=1 -> {:?}", second.is_ok())
            },
            |store| expect("node.x", store.get::<u32>(["cfg", "node", "x"]), None),
        ),
        scripted(
            "table replaced by a scalar",
            |store| {
                store.set(["cfg", "node", "x"], &1u32).unwrap();
                let second = store.set(["cfg", "node"], &2u32);
                format!("cfg.node.x=1 then cfg.node=2 -> {:?}", second.is_ok())
            },
            |store| expect("node.x", store.get::<u32>(["cfg", "node", "x"]), Some(1u32)),
        ),
        scripted(
            "two tables interleaved with scalars",
            |store| {
                store.set(["a", "t", "x"], &1u32).unwrap();
                store.set(["b"], &2u32).unwrap();
                store.set(["a", "u", "y"], &3u32).unwrap();
                store.set(["c"], &4u32).unwrap();
                "a.t.x, b, a.u.y, c".into()
            },
            |store| {
                join(&[
                    expect("a.t.x", store.get::<u32>(["a", "t", "x"]), Some(1u32)),
                    expect("b", store.get::<u32>(["b"]), Some(2u32)),
                    expect("a.u.y", store.get::<u32>(["a", "u", "y"]), Some(3u32)),
                    expect("c", store.get::<u32>(["c"]), Some(4u32)),
                ])
            },
        ),
        scripted(
            "a whole struct, then a key beside it",
            |store| {
                store
                    .set(
                        ["cfg"],
                        &Mixed {
                            scalar: 1,
                            table: Inner { x: 2 },
                            after: 3,
                        },
                    )
                    .unwrap();
                store.set(["cfg", "extra"], &9u32).unwrap();
                "cfg=<Mixed> then cfg.extra=9".into()
            },
            |store| {
                join(&[
                    expect("scalar", store.get::<u32>(["cfg", "scalar"]), Some(1u32)),
                    expect(
                        "table.x",
                        store.get::<u32>(["cfg", "table", "x"]),
                        Some(2u32),
                    ),
                    expect("after", store.get::<u32>(["cfg", "after"]), Some(3u32)),
                    expect("extra", store.get::<u32>(["cfg", "extra"]), Some(9u32)),
                ])
            },
        ),
        scripted(
            "array of tables, then a key beside it",
            |store| {
                store
                    .set(
                        ["servers"],
                        &vec![Server { name: "a".into() }, Server { name: "b".into() }],
                    )
                    .unwrap();
                let beside = store.set(["servers", "count"], &2u32);
                format!("servers=[..] then servers.count -> {:?}", beside.is_ok())
            },
            |store| {
                expect(
                    "servers",
                    store.get::<Vec<Server>>(["servers"]),
                    Some(vec![
                        Server { name: "a".into() },
                        Server { name: "b".into() },
                    ]),
                )
            },
        ),
        scripted(
            "delete inside a written struct",
            |store| {
                store
                    .set(
                        ["cfg"],
                        &Mixed {
                            scalar: 1,
                            table: Inner { x: 2 },
                            after: 3,
                        },
                    )
                    .unwrap();
                store.delete(["cfg", "scalar"]).unwrap();
                "cfg=<Mixed> then delete cfg.scalar".into()
            },
            |store| expect("scalar", store.get::<u32>(["cfg", "scalar"]), None),
        ),
        scripted(
            "write None over an existing value",
            |store| {
                store.set(["cfg", "maybe"], &Some(1u32)).unwrap();
                let second = store.set(["cfg", "maybe"], &Option::<u32>::None);
                format!("Some(1) then None -> {:?}", second.is_ok())
            },
            |store| expect("maybe", store.get::<u32>(["cfg", "maybe"]), None),
        ),
        scripted(
            "sibling of a dotted-key segment",
            |store| {
                store.set(["cfg", "a.b"], &1u32).unwrap();
                store.set(["cfg", "a"], &2u32).unwrap();
                "cfg.'a.b'=1 then cfg.a=2".into()
            },
            |store| {
                join(&[
                    expect("a.b", store.get::<u32>(["cfg", "a.b"]), Some(1u32)),
                    expect("a", store.get::<u32>(["cfg", "a"]), Some(2u32)),
                ])
            },
        ),
    ];
    table("7. ordering and table placement", &rows);
}

fn join(parts: &[String]) -> String {
    if parts.iter().all(|p| p.starts_with("OK")) {
        format!("OK ({} checks)", parts.len())
    } else {
        parts
            .iter()
            .filter(|p| !p.starts_with("OK"))
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A value that nests `depth` arrays deep and counts them back on the way in.
#[derive(Debug, Clone, PartialEq)]
struct Nest(u32);

impl Serialize for Nest {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        if self.0 == 0 {
            return s.serialize_u32(0);
        }
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(&Nest(self.0 - 1))?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Nest {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Nest;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a nest of arrays around an integer")
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Nest, E> {
                Ok(Nest(0))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Nest, E> {
                Ok(Nest(0))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Nest, A::Error> {
                let inner: Nest = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("empty nest"))?;
                Ok(Nest(inner.0 + 1))
            }
        }
        d.deserialize_any(V)
    }
}

#[test]
fn depth_of_a_value() {
    let mut rows = Vec::new();
    for depth in [1u32, 8, 16, 24, 32, 40, 48, 56, 64, 96, 128] {
        rows.push(probe(
            &format!("value nested {depth} deep"),
            &["d", "leaf"],
            Nest(depth),
        ));
    }
    let trimmed: Vec<Row> = rows
        .into_iter()
        .map(|mut r| {
            r.file.clear();
            r.keys.clear();
            r
        })
        .collect();
    table("8a. value depth", &trimmed);
}

#[test]
fn depth_of_a_path() {
    let mut rows = Vec::new();
    for depth in [1usize, 8, 32, 64, 128, 256, 512] {
        let segs: Vec<String> = (0..depth).map(|i| format!("s{i}")).collect();
        let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
        rows.push(probe(&format!("path {depth} levels deep"), &refs, 42u32));
    }
    let trimmed: Vec<Row> = rows
        .into_iter()
        .map(|mut r| {
            r.file.clear();
            r.keys.clear();
            r
        })
        .collect();
    table("8b. path depth", &trimmed);
}

#[test]
fn depth_of_both() {
    let mut rows = Vec::new();
    for (path_depth, value_depth) in [(2usize, 40u32), (16, 40), (64, 40), (2, 100), (16, 100)] {
        let segs: Vec<String> = (0..path_depth).map(|i| format!("s{i}")).collect();
        let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
        rows.push(probe(
            &format!("path {path_depth} deep, value {value_depth} deep"),
            &refs,
            Nest(value_depth),
        ));
    }
    let trimmed: Vec<Row> = rows
        .into_iter()
        .map(|mut r| {
            r.file.clear();
            r.keys.clear();
            r
        })
        .collect();
    table("8c. path and value depth together", &trimmed);
}

/// The same nesting as [`Nest`], spelled with tables instead of arrays.
#[derive(Debug, Clone, PartialEq)]
struct NestTable(u32);

impl Serialize for NestTable {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        if self.0 == 0 {
            return s.serialize_u32(0);
        }
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry("n", &NestTable(self.0 - 1))?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for NestTable {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = NestTable;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a nest of tables around an integer")
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<NestTable, E> {
                Ok(NestTable(0))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<NestTable, E> {
                Ok(NestTable(0))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<NestTable, A::Error> {
                let entry: Option<(String, NestTable)> = map.next_entry()?;
                let (_, inner) =
                    entry.ok_or_else(|| serde::de::Error::custom("empty nest of tables"))?;
                Ok(NestTable(inner.0 + 1))
            }
        }
        d.deserialize_any(V)
    }
}

/// The largest `n` that still round-trips, and the smallest that does not.
fn boundary(label: &str, lo: usize, hi: usize, mut at: impl FnMut(usize) -> Verdict) -> String {
    let first = at(lo);
    if first != Verdict::Faithful {
        return format!("{label}: already {} at {lo}", first.tag());
    }
    let last = at(hi);
    if last == Verdict::Faithful {
        return format!("{label}: still faithful at {hi}");
    }

    let (mut good, mut bad) = (lo, hi);
    while bad - good > 1 {
        let mid = good + (bad - good) / 2;
        if at(mid) == Verdict::Faithful {
            good = mid;
        } else {
            bad = mid;
        }
    }
    format!(
        "{label}: last faithful {good}, first failure {bad} -> {}",
        at(bad).tag()
    )
}

fn path_of(depth: usize) -> Vec<String> {
    (0..depth).map(|i| format!("s{i}")).collect()
}

#[test]
fn depth_boundaries() {
    println!("\n### 8d. where depth stops working");

    println!(
        "  {}",
        boundary("path levels, scalar leaf", 1, 200, |n| {
            let segs = path_of(n);
            let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
            probe("depth_path", &refs, 42u32).verdict
        })
    );

    println!(
        "  {}",
        boundary("value nesting, two-level path", 1, 200, |n| {
            probe("depth_value", &["d", "leaf"], Nest(n as u32)).verdict
        })
    );

    println!(
        "  {}",
        boundary("inline-table nesting, two-level path", 1, 200, |n| {
            probe("depth_table", &["d", "leaf"], NestTable(n as u32)).verdict
        })
    );

    for value_depth in [40u32, 60] {
        println!(
            "  {}",
            boundary(
                &format!("path levels with an inline table nested {value_depth} deep"),
                1,
                200,
                |n| {
                    let segs = path_of(n);
                    let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
                    probe("depth_both_table", &refs, NestTable(value_depth)).verdict
                }
            )
        );
    }

    for value_depth in [1u32, 16, 32, 60] {
        println!(
            "  {}",
            boundary(
                &format!("path levels with a value nested {value_depth} deep"),
                1,
                200,
                |n| {
                    let segs = path_of(n);
                    let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
                    probe("depth_both", &refs, Nest(value_depth)).verdict
                }
            )
        );
    }
}

/// What one path too deep costs the rest of the file.
#[test]
fn a_deep_path_takes_the_whole_file_with_it() {
    println!("\n### 8e. what the deep path costs");

    let tmp = temp_for("deep_consequence");
    let segs = path_of(120);
    let refs: Vec<&str> = segs.iter().map(String::as_str).collect();

    {
        let store = open(tmp.path()).unwrap();
        store.set(["important", "setting"], &1280u32).unwrap();
        store.save_now().unwrap();

        let wrote = store.set(refs.clone(), &42u32);
        let flushed = store.save_now();
        println!(
            "  the deep write returned {:?} and the flush {:?}",
            wrote.is_ok(),
            flushed.is_ok()
        );

        let live = store.get::<u32>(["important", "setting"]);
        println!("  the live store still reads the earlier value: {live:?}");

        let later = store.set(["important", "setting"], &1920u32);
        println!(
            "  a write afterwards, which re-reads the file first: {:?}",
            later.as_ref().map_err(why)
        );
        let later_flush = store.save_now();
        println!("  and its flush: {:?}", later_flush.as_ref().map_err(why));
    }

    let on_disk = std::fs::read_to_string(tmp.path()).unwrap_or_default();
    println!("  the file is {} bytes", on_disk.len());
    let backup = {
        let mut name = tmp.path().file_name().unwrap().to_os_string();
        name.push(".bak");
        tmp.path().with_file_name(name)
    };
    println!("  a backup beside it: {}", backup.exists());

    match open(tmp.path()) {
        Ok(store) => {
            println!(
                "  reopened, and the earlier value reads {:?}",
                store.get::<u32>(["important", "setting"])
            );
        }
        Err(e) => println!("  the store will not open: {}", why(&e)),
    }
}

/// The same content, written two ways, read back at the parent.
///
/// A struct written whole becomes an inline table; the same keys written one
/// at a time become a `[section]`. Nothing the caller does distinguishes them.
#[test]
fn reading_a_parent() {
    let rows = vec![
        scripted(
            "parent written whole, read whole",
            |store| {
                store
                    .set(
                        ["cfg"],
                        &BTreeMap::from([("a".to_string(), 1u32), ("b".into(), 2)]),
                    )
                    .unwrap();
                "cfg={a=1,b=2} in one write".into()
            },
            |store| {
                expect(
                    "cfg",
                    store.get::<BTreeMap<String, u32>>(["cfg"]),
                    Some(BTreeMap::from([("a".to_string(), 1u32), ("b".into(), 2)])),
                )
            },
        ),
        scripted(
            "parent written key by key, read whole",
            |store| {
                store.set(["cfg", "a"], &1u32).unwrap();
                store.set(["cfg", "b"], &2u32).unwrap();
                "cfg.a=1 and cfg.b=2 in two writes".into()
            },
            |store| {
                expect(
                    "cfg",
                    store.get::<BTreeMap<String, u32>>(["cfg"]),
                    Some(BTreeMap::from([("a".to_string(), 1u32), ("b".into(), 2)])),
                )
            },
        ),
        scripted(
            "a nested parent written key by key",
            |store| {
                store.set(["cfg", "inner", "x"], &1u32).unwrap();
                "cfg.inner.x=1".into()
            },
            |store| {
                expect(
                    "cfg.inner",
                    store.get::<BTreeMap<String, u32>>(["cfg", "inner"]),
                    Some(BTreeMap::from([("x".to_string(), 1u32)])),
                )
            },
        ),
        scripted(
            "a parent whose first key is a table",
            |store| {
                store.set(["cfg", "inner", "x"], &1u32).unwrap();
                store.set(["cfg", "flat"], &2u32).unwrap();
                "cfg.inner.x=1 then cfg.flat=2".into()
            },
            |store| {
                expect(
                    "cfg",
                    store.get::<Nested>(["cfg"]),
                    Some(Nested {
                        flat: 2,
                        inner: Inner { x: 1 },
                    }),
                )
            },
        ),
        scripted(
            "a struct read back after a key was added beside it",
            |store| {
                store
                    .set(["cfg"], &BTreeMap::from([("a".to_string(), 1u32)]))
                    .unwrap();
                store.set(["cfg", "b"], &2u32).unwrap();
                "cfg={a=1} then cfg.b=2".into()
            },
            |store| {
                expect(
                    "cfg",
                    store.get::<BTreeMap<String, u32>>(["cfg"]),
                    Some(BTreeMap::from([("a".to_string(), 1u32), ("b".into(), 2)])),
                )
            },
        ),
        scripted(
            "a one-key section read as a scalar",
            |store| {
                store.set(["cfg", "width", "px"], &800u16).unwrap();
                "cfg.width.px=800".into()
            },
            |store| expect("cfg.width", store.get::<u16>(["cfg", "width"]), None),
        ),
        scripted(
            "a one-key section read as the wrong scalar type",
            |store| {
                store
                    .set(["cfg", "width", "px"], &"eight".to_string())
                    .unwrap();
                "cfg.width.px=\"eight\"".into()
            },
            |store| expect("cfg.width", store.get::<String>(["cfg", "width"]), None),
        ),
        scripted(
            "the whole document read at the root",
            |store| {
                store.set(["cfg", "a"], &1u32).unwrap();
                "cfg.a=1".into()
            },
            |store| {
                let read =
                    store.get::<BTreeMap<String, BTreeMap<String, u32>>>(Vec::<String>::new());
                format!("root reads {:?}", read.map_err(|e| why(&e)))
            },
        ),
    ];
    table("11. reading a parent", &rows);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Nested {
    flat: u32,
    inner: Inner,
}

/// Width, where depth failed - a table with a great many siblings.
#[test]
fn width_of_a_table() {
    println!("\n### 8f. width");
    for count in [100usize, 10_000] {
        let tmp = temp_for(&format!("width{count}"));
        {
            let store = open(tmp.path()).unwrap();
            for i in 0..count {
                store.set(["wide", &format!("k{i}")], &(i as u32)).unwrap();
            }
            store.save_now().unwrap();
        }
        match open(tmp.path()) {
            Ok(store) => println!(
                "  {count} siblings: reopened, last reads {:?}",
                store.get::<u32>(["wide", &format!("k{}", count - 1)])
            ),
            Err(e) => println!("  {count} siblings: will not open: {}", why(&e)),
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Dated {
    when: String,
}

#[test]
fn beyond_the_list() {
    let rows = vec![
        probe(
            "a map keyed by something not a string",
            &["x", "intmap"],
            BTreeMap::from([(1u32, "a".to_string()), (2, "b".into())]),
        ),
        probe(
            "a struct field named like a toml keyword",
            &["x", "dated"],
            Dated {
                when: "1979-05-27T07:32:00Z".into(),
            },
        ),
        probe(
            "vec of vecs of tables",
            &["x", "nested_aot"],
            vec![vec![Server { name: "a".into() }]],
        ),
        probe(
            "a table inside an array",
            &["x", "mixed_arr"],
            vec![Mixed {
                scalar: 1,
                table: Inner { x: 2 },
                after: 3,
            }],
        ),
        probe(
            "an Option table field that is None",
            &["x", "opt_table"],
            OptTable {
                head: 1,
                body: None,
            },
        ),
        probe(
            "an Option table field that is Some",
            &["x", "opt_table_some"],
            OptTable {
                head: 1,
                body: Some(Inner { x: 2 }),
            },
        ),
        probe(
            "a struct with a table field before a scalar field",
            &["x", "table_first"],
            TableFirst {
                table: Inner { x: 1 },
                scalar: 2,
            },
        ),
        probe(
            "a map whose key is a dotted path",
            &["x", "dotted_map"],
            BTreeMap::from([("a.b.c".to_string(), 1u32)]),
        ),
        probe(
            "a deeply keyed map of tables",
            &["x", "map_of_tables"],
            BTreeMap::from([
                ("one".to_string(), Inner { x: 1 }),
                ("two".to_string(), Inner { x: 2 }),
            ]),
        ),
    ];
    table("9. beyond the list", &rows);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct OptTable {
    head: u32,
    body: Option<Inner>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct TableFirst {
    table: Inner,
    scalar: u32,
}

#[test]
fn residue_and_neighbours() {
    let rows = vec![scripted(
        "does a write leave keys nobody wrote",
        |store| {
            store.set(["only", "one"], &1u32).unwrap();
            "only.one=1".into()
        },
        |store| {
            let keys = every_key(store);
            if keys == vec!["only.one".to_string()] {
                "OK just the one key".into()
            } else {
                format!("keys are {keys:?}")
            }
        },
    )];
    table("10a. residue", &rows);
}

#[test]
fn residue_of_a_dotted_segment() {
    let rows = vec![scripted(
        "does an awkward segment come back as its own path",
        |store| {
            store.set(["out", "a.b"], &1u32).unwrap();
            "out.'a.b'=1".into()
        },
        |store| {
            let keys = every_key(store);
            let direct = store.get::<u32>(["out", "a.b"]);
            let split = store.get::<u32>(["out", "a", "b"]);
            format!("keys {keys:?}; direct {direct:?}; split {split:?}")
        },
    )];
    table("10a2. residue", &rows);
}

#[test]
fn residue_of_a_newline_segment() {
    let rows = vec![scripted(
        "does a newline in a segment come back",
        |store| {
            store.set(["out", "a\nb"], &1u32).unwrap();
            "out.'a\\nb'=1".into()
        },
        |store| {
            let keys = every_key(store);
            let direct = store.get::<u32>(["out", "a\nb"]);
            format!("keys {keys:?}; direct {direct:?}")
        },
    )];
    table("10a. residue", &rows);
}

#[test]
fn residue_of_emptiness() {
    let rows = vec![
        scripted(
            "does writing None leave the table behind",
            |store| {
                store.set(["gone", "leaf"], &Option::<u32>::None).unwrap();
                "gone.leaf=None".into()
            },
            |store| {
                let keys = every_key(store);
                format!("keys {keys:?}")
            },
        ),
        scripted(
            "does an empty struct leave an addressable table",
            |store| {
                store.set(["hollow"], &Empty {}).unwrap();
                "hollow={}".into()
            },
            |store| {
                let keys = every_key(store);
                let back = store.get::<Empty>(["hollow"]);
                format!("keys {keys:?}; back {back:?}")
            },
        ),
        scripted(
            "a map key no path can hold",
            |store| {
                store
                    .set(
                        ["holder"],
                        &BTreeMap::from([(String::new(), 1u32), ("kept".to_string(), 2u32)]),
                    )
                    .unwrap();
                "holder={ ''=1, kept=2 }".into()
            },
            |store| {
                let keys = every_key(store);
                let whole = store
                    .get::<BTreeMap<String, u32>>(["holder"])
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_else(|e| format!("Err {}", why(&e)));
                format!("keys {keys:?}; whole {whole}")
            },
        ),
    ];
    table("10b. residue", &rows);
}

#[test]
fn residue_of_awkward_keys() {
    let rows = vec![scripted(
        "a path written under a key holding a separator",
        |store| {
            store.set(["a.b"], &1u32).unwrap();
            store.set(["a", "b"], &2u32).unwrap();
            "'a.b'=1 then a.b=2".into()
        },
        |store| {
            join(&[
                expect("'a.b'", store.get::<u32>(["a.b"]), Some(1u32)),
                expect("a.b", store.get::<u32>(["a", "b"]), Some(2u32)),
            ])
        },
    )];
    table("10c. residue", &rows);
}

#[test]
fn residue_of_deletes() {
    let rows = vec![
        scripted(
            "deleting the last key of a table",
            |store| {
                store.set(["t", "only"], &1u32).unwrap();
                store.delete(["t", "only"]).unwrap();
                "t.only=1 then delete".into()
            },
            |store| {
                let keys = every_key(store);
                format!("keys {keys:?}")
            },
        ),
        scripted(
            "an empty vec is not an empty table",
            |store| {
                store.set(["e", "list"], &Vec::<u32>::new()).unwrap();
                store
                    .set(["e", "map"], &BTreeMap::<String, u32>::new())
                    .unwrap();
                "e.list=[] and e.map={}".into()
            },
            |store| {
                let keys = every_key(store);
                let list = store.get::<Vec<u32>>(["e", "list"]);
                let map = store.get::<BTreeMap<String, u32>>(["e", "map"]);
                format!("keys {keys:?}; list {list:?}; map {map:?}")
            },
        ),
        scripted(
            "is a leaf its own child",
            |store| {
                store.set(["leafy", "value"], &1u32).unwrap();
                "leafy.value=1".into()
            },
            |store| {
                let under_leaf = store
                    .scan_keys(["leafy", "value"])
                    .map(|k| k.iter().map(|p| p.as_str().to_string()).collect::<Vec<_>>())
                    .unwrap_or_else(|e| vec![format!("Err {}", why(&e))]);
                let under_absent = store
                    .scan_keys(["leafy", "nothing"])
                    .map(|k| k.iter().map(|p| p.as_str().to_string()).collect::<Vec<_>>())
                    .unwrap_or_else(|e| vec![format!("Err {}", why(&e))]);
                format!("scan of the leaf {under_leaf:?}; scan of an absent path {under_absent:?}")
            },
        ),
    ];
    table("10d. residue", &rows);
}
