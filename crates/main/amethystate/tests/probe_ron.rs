//! What the ron engine takes on a write and does not give back on a read.
//!
//! Every probe here does the same thing: open a store on `Backend::Ron`, write
//! one value at one path, flush, close, open the file again, read the same path
//! back, and compare. Nothing is asserted about the value - the point is to
//! record what happened, so the run prints a table and the reader decides.
//!
//! Run it as `cargo test -p amethystate --features ron --test probe_ron --
//! --nocapture`; without `--nocapture` the tables are swallowed and the file
//! says nothing.

#![cfg(feature = "ron")]

use amethystate::Store;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Debug};

/// Which of the failures a probe landed on, or none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Written, read back, equal.
    Kept,
    /// The write said no, which is the honest answer and not a defect.
    Refused,
    /// The path itself was refused before any store saw it.
    NoPath,
    /// The write said yes and the read disagrees, or finds nothing.
    Altered,
    /// The file or the store will not open afterwards.
    Opaque,
    /// The value came back, and paths nobody wrote came back with it.
    Residue,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Kept => "kept",
            Class::Refused => "write refused",
            Class::NoPath => "path refused",
            Class::Altered => "SILENT ALTERATION",
            Class::Opaque => "OPAQUE FAILURE",
            Class::Residue => "RESIDUE",
        }
    }
}

struct Row {
    label: String,
    wrote: String,
    write: String,
    back: String,
    class: Class,
    note: String,
}

impl Row {
    fn new(label: &str, wrote: String, write: &str, back: String, class: Class) -> Self {
        Self {
            label: label.to_string(),
            wrote,
            write: write.to_string(),
            back,
            class,
            note: String::new(),
        }
    }

    fn noting(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

fn table(title: &str, rows: &[Row]) {
    println!("\n=== {title} ===");
    for row in rows {
        println!(
            "[{}] {}\n    wrote: {}\n    write: {}\n    read : {}{}",
            row.class.label(),
            row.label,
            row.wrote,
            row.write,
            row.back,
            if row.note.is_empty() {
                String::new()
            } else {
                format!("\n    note : {}", row.note)
            }
        );
    }
}

fn cut(s: &str, at: usize) -> String {
    let escaped: String = s
        .chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            c if (c as u32) < 0x20 => format!("\\u{{{:x}}}", c as u32),
            c => c.to_string(),
        })
        .collect();
    if escaped.chars().count() > at {
        let head: String = escaped.chars().take(at).collect();
        format!("{head}... ({} chars)", escaped.chars().count())
    } else {
        escaped
    }
}

fn why<C>(report: &error_stack::Report<C>) -> String {
    cut(&format!("{report}").replace('\n', " "), 140)
}

fn open(at: &std::path::Path) -> Result<Store, String> {
    StoreBuilder::new(at)
        .backend(Backend::Ron)
        .build()
        .map_err(|e| why(&e))
}

/// Writes `value` at `segments`, closes, reopens, reads it back.
fn probe<T>(label: &str, segments: &[&str], value: &T) -> Row
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let wrote = cut(&format!("{value:?}"), 90);

    let path = match StorePath::try_from_segments(segments) {
        Ok(path) => path,
        Err(e) => {
            return Row::new(
                label,
                wrote,
                &format!("n/a: {e}"),
                "n/a".into(),
                Class::NoPath,
            );
        }
    };

    let file = TempPath::new("probe_ron");

    {
        let store = match open(file.path()) {
            Ok(store) => store,
            Err(e) => {
                return Row::new(
                    label,
                    wrote,
                    "n/a",
                    format!("fresh store: {e}"),
                    Class::Opaque,
                );
            }
        };
        if let Err(e) = store.set(&path, value) {
            return Row::new(
                label,
                wrote,
                &format!("Err: {}", why(&e)),
                "n/a".into(),
                Class::Refused,
            );
        }
        if let Err(e) = store.save_now() {
            return Row::new(
                label,
                wrote,
                &format!("Ok, flush Err: {}", why(&e)),
                "n/a".into(),
                Class::Refused,
            );
        }
    }

    let on_disk =
        std::fs::read_to_string(file.path()).unwrap_or_else(|e| format!("<unreadable: {e}>"));

    let store = match open(file.path()) {
        Ok(store) => store,
        Err(e) => {
            return Row::new(
                label,
                wrote,
                "Ok",
                format!("reopen failed: {e}"),
                Class::Opaque,
            )
            .noting(format!("file: {}", cut(&on_disk, 200)));
        }
    };

    let children = match store.scan_keys(&path) {
        Ok(keys) => keys
            .iter()
            .map(|k| k.as_str().to_string())
            .filter(|k| k != path.as_str())
            .collect::<Vec<_>>(),
        Err(e) => vec![format!("<scan failed: {}>", why(&e))],
    };

    let siblings = match path.parent() {
        Some(parent) => match store.scan_keys(&parent) {
            Ok(keys) => keys
                .iter()
                .map(|k| k.as_str().to_string())
                .collect::<Vec<_>>(),
            Err(e) => vec![format!("<scan failed: {}>", why(&e))],
        },
        None => Vec::new(),
    };

    let stray = !siblings.is_empty() && !siblings.iter().any(|k| k == path.as_str());

    match store.get::<T>(&path) {
        Err(e) => Row::new(
            label,
            wrote,
            "Ok",
            format!("read Err: {}", why(&e)),
            Class::Altered,
        )
        .noting(format!("file: {}", cut(&on_disk, 200))),
        Ok(None) => Row::new(
            label,
            wrote,
            "Ok",
            "nothing at the path".into(),
            Class::Altered,
        )
        .noting(format!(
            "file: {}, level holds: {siblings:?}",
            cut(&on_disk, 200)
        )),
        Ok(Some(back)) if back == *value => {
            if !children.is_empty() {
                Row::new(label, wrote, "Ok", "equal".into(), Class::Residue)
                    .noting(format!("readable under it: {children:?}"))
            } else if stray {
                Row::new(label, wrote, "Ok", "equal".into(), Class::Residue).noting(format!(
                    "the level holds {siblings:?}, not the path written"
                ))
            } else {
                Row::new(label, wrote, "Ok", "equal".into(), Class::Kept)
            }
        }
        Ok(Some(back)) => Row::new(
            label,
            wrote,
            "Ok",
            cut(&format!("{back:?}"), 90),
            Class::Altered,
        )
        .noting(format!("file: {}", cut(&on_disk, 200))),
    }
}

fn at(label: &str) -> Vec<&str> {
    vec!["probe", label]
}

// --- the types the probes are made of -------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
enum Mode {
    #[default]
    Off,
    On,
    Level(u8),
    Pair(u8, u8),
    Named {
        a: u8,
        b: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
enum Untagged {
    Number(u32),
    Text(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind")]
enum Tagged {
    First { a: u8 },
    Second { b: u8 },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", content = "with")]
enum Adjacent {
    Unit,
    Pair(u8, u8),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Holder {
    mode: Mode,
    n: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct UnitStruct;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Newtype(u32);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct TupleStruct(u32, String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct EmptyStruct {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Level5 {
    a: Level4,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Level4 {
    b: Level3,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Level3 {
    c: Level2,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Level2 {
    d: Level1,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Level1 {
    e: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Inner {
    x: u8,
    y: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Flat {
    top: u8,
    #[serde(flatten)]
    rest: BTreeMap<String, u8>,
}

/// An `f64` compared by its bits, so `-0.0`, `0.0` and the two `NaN`s are told
/// apart rather than declared equal or unequal by IEEE rules.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct Bits(f64);

impl PartialEq for Bits {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Debug for Bits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{:#018x}>", self.0, self.0.to_bits())
    }
}

/// A value nested `0` levels deep, as a sequence inside a sequence.
#[derive(Debug, Clone, PartialEq)]
struct Deep(u32);

impl Serialize for Deep {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        if self.0 == 0 {
            return s.serialize_u32(7);
        }
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(&Deep(self.0 - 1))?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Deep {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Deep;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number wrapped in some number of sequences")
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Deep, E> {
                Ok(Deep(0))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Deep, E> {
                Ok(Deep(0))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Deep, E> {
                Ok(Deep(0))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Deep, A::Error> {
                let inner: Deep = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("an empty sequence"))?;
                Ok(Deep(inner.0 + 1))
            }
        }
        d.deserialize_any(V)
    }
}

// --- the probes ------------------------------------------------------------

#[test]
fn enums() {
    let rows = vec![
        probe("unit variant", &at("unit_variant"), &Mode::On),
        probe(
            "unit variant (the default one)",
            &at("unit_default"),
            &Mode::Off,
        ),
        probe("newtype variant", &at("newtype_variant"), &Mode::Level(3)),
        probe("tuple variant", &at("tuple_variant"), &Mode::Pair(1, 2)),
        probe(
            "struct variant",
            &at("struct_variant"),
            &Mode::Named {
                a: 1,
                b: "x".into(),
            },
        ),
        probe(
            "enum in a struct field",
            &at("enum_in_struct"),
            &Holder {
                mode: Mode::On,
                n: 4,
            },
        ),
        probe(
            "enum in a Vec",
            &at("enum_in_vec"),
            &vec![Mode::On, Mode::Level(2)],
        ),
        probe(
            "enum as a map value",
            &at("enum_in_map"),
            &BTreeMap::from([("k".to_string(), Mode::On)]),
        ),
        probe("enum in Some", &at("enum_in_some"), &Some(Mode::On)),
        probe("enum in a tuple", &at("enum_in_tuple"), &(1u8, Mode::On)),
        probe("Option<Mode>::None", &at("enum_none"), &None::<Mode>),
        probe("std Result::Ok", &at("result_ok"), &Ok::<u8, String>(1)),
        probe(
            "std Result::Err",
            &at("result_err"),
            &Err::<u8, String>("no".into()),
        ),
        probe("untagged enum", &at("untagged"), &Untagged::Number(5)),
        probe(
            "untagged enum, string arm",
            &at("untagged_s"),
            &Untagged::Text("hi".into()),
        ),
        probe(
            "internally tagged enum",
            &at("tagged"),
            &Tagged::First { a: 9 },
        ),
        probe(
            "adjacently tagged, unit arm",
            &at("adjacent_unit"),
            &Adjacent::Unit,
        ),
        probe(
            "adjacently tagged, tuple arm",
            &at("adjacent_pair"),
            &Adjacent::Pair(1, 2),
        ),
    ];
    table("enums", &rows);
}

#[test]
fn structs_and_tuples() {
    let rows = vec![
        probe("unit struct", &at("unit_struct"), &UnitStruct),
        probe("newtype struct", &at("newtype_struct"), &Newtype(7)),
        probe(
            "tuple struct",
            &at("tuple_struct"),
            &TupleStruct(7, "x".into()),
        ),
        probe("empty struct", &at("empty_struct"), &EmptyStruct {}),
        probe(
            "five structs deep",
            &at("nested_struct"),
            &Level5 {
                a: Level4 {
                    b: Level3 {
                        c: Level2 { d: Level1 { e: 1 } },
                    },
                },
            },
        ),
        probe("plain struct", &at("plain_struct"), &Inner { x: 1, y: 2 }),
        probe(
            "flattened map in a struct",
            &at("flatten"),
            &Flat {
                top: 1,
                rest: BTreeMap::from([("a".to_string(), 2u8)]),
            },
        ),
        probe("unit ()", &at("unit"), &()),
        probe("one-element tuple", &at("tuple1"), &(7u8,)),
        probe("two-element tuple", &at("tuple2"), &(7u8, 8u8)),
        probe(
            "twelve-element tuple",
            &at("tuple12"),
            &(
                1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, 10u8, 11u8, 12u8,
            ),
        ),
        probe(
            "tuples in a Vec",
            &at("tuple_vec"),
            &vec![(1u8, 2u8), (3, 4)],
        ),
        probe("empty Vec", &at("empty_vec"), &Vec::<u8>::new()),
        probe(
            "empty map",
            &at("empty_map"),
            &BTreeMap::<String, u8>::new(),
        ),
        probe(
            "map with numeric keys",
            &at("numeric_keys"),
            &BTreeMap::from([(1u32, "a".to_string())]),
        ),
        probe(
            "map with a dotted key",
            &at("dotted_map_key"),
            &BTreeMap::from([("a.b".to_string(), 1u8)]),
        ),
        probe(
            "map with an empty key",
            &at("empty_map_key"),
            &BTreeMap::from([(String::new(), 1u8)]),
        ),
        probe(
            "struct in a Vec",
            &at("struct_vec"),
            &vec![Inner { x: 1, y: 2 }],
        ),
    ];
    table("structs, tuples, collections", &rows);
}

#[test]
fn numbers() {
    let rows = vec![
        probe("f64::NAN", &at("nan"), &Bits(f64::NAN)),
        probe("-f64::NAN", &at("neg_nan"), &Bits(-f64::NAN)),
        probe("f64::INFINITY", &at("inf"), &Bits(f64::INFINITY)),
        probe(
            "f64::NEG_INFINITY",
            &at("neg_inf"),
            &Bits(f64::NEG_INFINITY),
        ),
        probe("-0.0", &at("neg_zero"), &Bits(-0.0)),
        probe("0.0", &at("zero"), &Bits(0.0)),
        probe("1.0", &at("one_point_zero"), &Bits(1.0)),
        probe("0.1 + 0.2", &at("precision"), &Bits(0.1 + 0.2)),
        probe(
            "f64::MIN_POSITIVE",
            &at("min_positive"),
            &Bits(f64::MIN_POSITIVE),
        ),
        probe("f64::MAX", &at("f64_max"), &Bits(f64::MAX)),
        probe("5e-324 (subnormal)", &at("subnormal"), &Bits(5e-324)),
        probe("f32::NAN", &at("f32_nan"), &f32::NAN.to_bits()),
        probe("u64::MAX", &at("u64_max"), &u64::MAX),
        probe("i64::MIN", &at("i64_min"), &i64::MIN),
        probe("u128::MAX", &at("u128_max"), &u128::MAX),
        probe("i128::MIN", &at("i128_min"), &i128::MIN),
        probe("u8 253", &at("u8"), &253u8),
        probe("char 'a'", &at("char"), &'a'),
        probe("char NUL", &at("char_nul"), &'\u{0}'),
        probe("char '\\u{10FFFF}'", &at("char_max"), &'\u{10FFFF}'),
        probe("bool", &at("bool"), &true),
        probe("bytes as Vec<u8>", &at("bytes"), &vec![0u8, 1, 255]),
    ];
    table("numbers, floats, scalars", &rows);
}

#[test]
fn strings() {
    let control: String = (1u8..0x20).map(char::from).collect();
    let long = "x".repeat(200_000);
    let rows = vec![
        probe("empty string", &at("empty_string"), &String::new()),
        probe("embedded NUL", &at("nul"), &"a\0b".to_string()),
        probe("every C0 control", &at("controls"), &control),
        probe("CRLF", &at("crlf"), &"a\r\nb".to_string()),
        probe("lone CR", &at("cr"), &"a\rb".to_string()),
        probe("200k characters", &at("long_string"), &long),
        probe(
            "unicode",
            &at("unicode"),
            &"日本語 🎉 \u{202e}rtl".to_string(),
        ),
        probe("ron enum syntax", &at("ron_enum"), &"Some(1)".to_string()),
        probe(
            "ron struct syntax",
            &at("ron_struct"),
            &"(a: 1)".to_string(),
        ),
        probe(
            "ron line comment",
            &at("ron_line_comment"),
            &"// nope".to_string(),
        ),
        probe(
            "ron block comment",
            &at("ron_block_comment"),
            &"/* nope */".to_string(),
        ),
        probe(
            "quotes and backslashes",
            &at("quotes"),
            &"he said \"hi\" \\ end".to_string(),
        ),
        probe(
            "raw string terminator",
            &at("raw_terminator"),
            &"\"#".to_string(),
        ),
        probe(
            "a string that reads as a number",
            &at("numeric_string"),
            &"1".to_string(),
        ),
        probe(
            "a string that reads as a keyword",
            &at("keyword_string"),
            &"true".to_string(),
        ),
        probe("a lone quote", &at("lone_quote"), &"\"".to_string()),
    ];
    table("strings", &rows);
}

#[test]
fn path_segments() {
    let long = "s".repeat(100_000);
    let rows = vec![
        probe("ordinary", &["probe", "plain"], &1u8),
        probe("empty segment", &["probe", ""], &1u8),
        probe("segment \".\"", &["probe", "."], &1u8),
        probe("segment \"..\"", &["probe", ".."], &1u8),
        probe("segment with a dot", &["probe", "a.b"], &1u8),
        probe("segment with a backslash", &["probe", "a\\b"], &1u8),
        probe("segment with a backslash-dot", &["probe", "a\\.b"], &1u8),
        probe("segment with a newline", &["probe", "a\nb"], &1u8),
        probe("segment with a quote", &["probe", "a\"b"], &1u8),
        probe("segment with a NUL", &["probe", "a\0b"], &1u8),
        probe("segment in unicode", &["probe", "日本語"], &1u8),
        probe("segment 100k long", &["probe", &long], &1u8),
        probe("segment named __init", &["__init", "probe"], &1u8),
        probe("segment named like a meta kind", &["__meta", "probe"], &1u8),
        probe("segment that is ron syntax", &["probe", "Some(1)"], &1u8),
        probe("segment that is a comment", &["probe", "// x"], &1u8),
        probe("single segment at the top", &["only"], &1u8),
    ];
    table("path segments", &rows);
}

#[test]
fn options() {
    let rows = vec![
        probe("None::<u8>", &at("none_u8"), &None::<u8>),
        probe("Some(5u8)", &at("some_u8"), &Some(5u8)),
        probe("Some(None::<u8>)", &at("some_none"), &Some(None::<u8>)),
        probe("Some(Some(5u8))", &at("some_some"), &Some(Some(5u8))),
        probe(
            "None::<Option<u8>>",
            &at("none_option"),
            &None::<Option<u8>>,
        ),
        probe("None::<String>", &at("none_string"), &None::<String>),
        probe("Vec of Options", &at("vec_option"), &vec![Some(1u8), None]),
        probe(
            "None in a struct field",
            &at("none_field"),
            &OptionField { a: None, b: 1 },
        ),
        probe(
            "Some(()) - the shape unit and none share",
            &at("some_unit"),
            &Some(()),
        ),
    ];
    table("options", &rows);
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct OptionField {
    a: Option<u8>,
    b: u8,
}

/// Two arms of an untagged enum that write the same text.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
enum Overlapping {
    Small(u32),
    Big(i64),
}

/// What the typed handle - the surface an application actually uses - does with
/// a value the engine degrades.
#[test]
fn handles() {
    use amethystate::store::field_with_path;
    use amethystate::uuid::Uuid;

    let rows = vec![probe(
        "an untagged enum whose arms overlap",
        &at("overlapping"),
        &Overlapping::Big(5),
    )];
    table("overlapping arms", &rows);

    println!("\n=== the typed handle ===");
    let file = TempPath::new("probe_ron_field");
    let store = open(file.path()).unwrap();

    match field_with_path::<Mode>(&store, ["probe", "mode"], Mode::Off, Uuid::new_v4()) {
        Err(e) => {
            let _ = store.save_now();
            println!(
                "declaring a Mode field: Err: {}, and the file holds {}",
                why(&e),
                cut(
                    &std::fs::read_to_string(file.path()).unwrap_or_default(),
                    200
                )
            );
        }
        Ok(field) => {
            println!("declaring a Mode field: Ok, holding {:?}", field.get());
            let set = field.set(Mode::Level(3));
            store.save_now().unwrap();
            println!(
                "setting it to Level(3): {:?}, the handle now holds {:?}, \
                 a typed read of the path says {:?}, the file says {}",
                set.map_err(|e| why(&e)),
                field.get(),
                store.get::<Mode>(["probe", "mode"]).map_err(|e| why(&e)),
                cut(
                    &std::fs::read_to_string(file.path()).unwrap_or_default(),
                    200
                )
            );
        }
    }
}

/// The same enums on json, to say whether the loss is ron's or the store's.
///
/// Inert unless the json feature is on: `cargo test -p amethystate --features
/// ron,json --test probe_ron contrast_with_json`.
#[cfg(feature = "json")]
#[test]
fn contrast_with_json() {
    fn json_probe<T>(label: &str, key: &str, value: &T) -> String
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        let file = TempPath::new("probe_json");
        let path = StorePath::from_segments(["probe", key]);
        {
            let store = StoreBuilder::new(file.path())
                .backend(Backend::Json)
                .build()
                .unwrap();
            if let Err(e) = store.set(&path, value) {
                return format!("{label}: write Err: {}", why(&e));
            }
            store.save_now().unwrap();
        }
        let store = match StoreBuilder::new(file.path())
            .backend(Backend::Json)
            .build()
        {
            Ok(store) => store,
            Err(e) => return format!("{label}: reopen Err: {}", why(&e)),
        };
        match store.get::<T>(&path) {
            Err(e) => format!("{label}: read Err: {}", why(&e)),
            Ok(None) => format!("{label}: nothing at the path"),
            Ok(Some(back)) if back == *value => format!("{label}: kept"),
            Ok(Some(back)) => format!("{label}: came back {back:?}"),
        }
    }

    println!("\n=== the same values on json ===");
    println!("{}", json_probe("unit variant", "unit", &Mode::On));
    println!(
        "{}",
        json_probe("newtype variant", "newtype", &Mode::Level(3))
    );
    println!(
        "{}",
        json_probe("tuple variant", "tuple", &Mode::Pair(1, 2))
    );
    println!(
        "{}",
        json_probe(
            "struct variant",
            "structv",
            &Mode::Named {
                a: 1,
                b: "x".into()
            }
        )
    );
    println!("{}", json_probe("empty struct", "empty", &EmptyStruct {}));
    println!("{}", json_probe("unit struct", "unitstruct", &UnitStruct));
    println!(
        "{}",
        json_probe("std Result", "result", &Ok::<u8, String>(1))
    );
}

/// Where the round trip stops working as the value or the path gets deeper.
#[test]
fn depth() {
    println!("\n=== depth ===");
    for path_depth in [2usize, 5, 10, 30, 60] {
        let boundary = deepest_that_round_trips(path_depth);
        println!(
            "path {path_depth} levels: deepest value that round-trips = {boundary:?}, \
             first failure = {:?}",
            deep_outcome(path_depth, boundary.map_or(0, |b| b + 1))
        );
    }

    for value_depth in (60usize..=70).chain([80, 100, 120, 127, 128, 129, 200]) {
        println!(
            "path 2 levels, value {value_depth} deep: {:?}",
            deep_outcome(2, value_depth)
        );
    }

    println!(
        "deepest value the write accepts at a 2-level path = {:?}",
        deepest_the_write_accepts(2)
    );
    println!(
        "deepest value the write accepts at a 30-level path = {:?}",
        deepest_the_write_accepts(30)
    );
}

fn deepest_the_write_accepts(path_depth: usize) -> usize {
    let mut low = 0usize;
    let mut high = 512usize;
    while high - low > 1 {
        let mid = (low + high) / 2;
        if deep_outcome(path_depth, mid) == DeepOutcome::WriteRefused {
            high = mid;
        } else {
            low = mid;
        }
    }
    low
}

#[derive(Debug, PartialEq, Eq)]
enum DeepOutcome {
    RoundTrips,
    WriteRefused,
    ReopenFailed,
    ReadFailed,
    CameBackDifferent,
}

fn deep_outcome(path_depth: usize, value_depth: usize) -> DeepOutcome {
    let segments: Vec<String> = (0..path_depth).map(|i| format!("l{i}")).collect();
    let path = StorePath::from_segments(&segments);
    let file = TempPath::new("probe_ron_depth");
    let value = Deep(value_depth as u32);

    {
        let Ok(store) = open(file.path()) else {
            return DeepOutcome::ReopenFailed;
        };
        if store.set(&path, &value).is_err() {
            return DeepOutcome::WriteRefused;
        }
        if store.save_now().is_err() {
            return DeepOutcome::WriteRefused;
        }
    }

    let Ok(store) = open(file.path()) else {
        return DeepOutcome::ReopenFailed;
    };
    match store.get::<Deep>(&path) {
        Err(_) => DeepOutcome::ReadFailed,
        Ok(Some(back)) if back == value => DeepOutcome::RoundTrips,
        Ok(_) => DeepOutcome::CameBackDifferent,
    }
}

fn deepest_that_round_trips(path_depth: usize) -> Option<usize> {
    if deep_outcome(path_depth, 0) != DeepOutcome::RoundTrips {
        return None;
    }
    let mut low = 0usize;
    let mut high = 512usize;
    if deep_outcome(path_depth, high) == DeepOutcome::RoundTrips {
        return Some(high);
    }
    while high - low > 1 {
        let mid = (low + high) / 2;
        if deep_outcome(path_depth, mid) == DeepOutcome::RoundTrips {
            low = mid;
        } else {
            high = mid;
        }
    }
    Some(low)
}

/// The rest of the category: things a store does with a value after it has
/// taken it, which a single write-and-read does not reach.
#[test]
fn beyond() {
    println!("\n=== beyond one write ===");

    struct_valued_leaf_is_addressable_by_path();
    a_write_under_a_struct_rewrites_the_struct();
    a_second_write_of_a_shallower_value();
    what_the_file_says_for_a_none();
}

fn struct_valued_leaf_is_addressable_by_path() {
    let file = TempPath::new("probe_ron_leafscan");
    let store = open(file.path()).unwrap();
    store.set(["probe", "cfg"], &Inner { x: 1, y: 2 }).unwrap();
    store.save_now().unwrap();

    let keys: Vec<String> = store
        .scan_keys(StorePath::from_segments(["probe"]))
        .unwrap()
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    let inner: Option<u8> = store.get(["probe", "cfg", "x"]).unwrap();

    println!(
        "a struct written at probe.cfg: scan of probe = {keys:?}, get(probe.cfg.x) = {inner:?}"
    );
}

fn a_write_under_a_struct_rewrites_the_struct() {
    let file = TempPath::new("probe_ron_overwrite");
    let store = open(file.path()).unwrap();
    store.set(["probe", "cfg"], &Inner { x: 1, y: 2 }).unwrap();
    let written = store.set(["probe", "cfg", "x"], &"not a number".to_string());
    store.save_now().unwrap();

    let whole: Result<Option<Inner>, String> = store.get(["probe", "cfg"]).map_err(|e| why(&e));
    println!(
        "writing a String at probe.cfg.x under a struct: write = {:?}, \
         reading probe.cfg as the struct = {:?}",
        written.map_err(|e| why(&e)),
        whole
    );
}

fn a_second_write_of_a_shallower_value() {
    let file = TempPath::new("probe_ron_shallower");
    let store = open(file.path()).unwrap();
    store.set(["probe", "a", "b"], &1u8).unwrap();
    let refused = store.set(["probe", "a"], &2u8);
    store.save_now().unwrap();
    println!(
        "writing a value at probe.a while probe.a.b exists: {:?}",
        refused.map_err(|e| why(&e))
    );
}

fn what_the_file_says_for_a_none() {
    let file = TempPath::new("probe_ron_none");
    {
        let store = open(file.path()).unwrap();
        store.set(["probe", "maybe"], &None::<u8>).unwrap();
        store.set(["probe", "other"], &1u8).unwrap();
        store.save_now().unwrap();
    }
    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let store = open(file.path()).unwrap();
    let keys: Vec<String> = store
        .scan_keys(StorePath::from_segments(["probe"]))
        .unwrap()
        .iter()
        .map(|k| k.as_str().to_string())
        .collect();
    println!(
        "a None written at probe.maybe: file = {}, scan of probe = {keys:?}",
        cut(&text, 200)
    );
}
