//! What redb takes on the way in and will not hand back on the way out.
//!
//! The text engines lose values to their formats: json cannot spell a
//! non-finite float, toml drops `None`, and a name holding an escape comes back
//! as a different name. redb has neither a document nor a text codec - values
//! go through msgpack and paths become keys in a table - so the question is
//! whether the same category has members here, and where they are.
//!
//! Every probe writes, flushes, drops the store, opens the file again and reads
//! the same path, because a value that survives only in the write buffer has
//! not been stored.

#![cfg(feature = "redb")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{StorageError, StorePath};
use amethystate::{StorageResult, Store};
use amethystate_core::test_utils::TempPath;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

/// A namespace named the way a key on disk spells it, so a name holding the
/// separator stays distinguishable from the two levels it looks like.
fn ns(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).expect("a namespace the test wrote itself")
}

/// A redb store with the debouncer and the watcher pushed out, so nothing
/// lands except where a probe asks for it.
fn open(file: &TempPath) -> Store {
    StoreBuilder::new(file.path())
        .backend(Backend::Redb)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .expect("the store opened")
}

/// What a write reported and what a read of the same path answered, with the
/// file closed and opened again in between.
struct Trip<T> {
    wrote: StorageResult<()>,
    read: StorageResult<Option<T>>,
}

impl<T> Trip<T> {
    fn value(self) -> Option<T> {
        self.wrote.expect("the write was refused");
        self.read.expect("the read failed")
    }
}

fn trip_at<T: Serialize + DeserializeOwned>(tag: &str, path: &StorePath, value: &T) -> Trip<T> {
    let file = TempPath::new(tag);

    let wrote = {
        let store = open(&file);
        let wrote = store.set(path, value);
        store
            .flush_prefix(StorePath::root())
            .expect("the flush landed");
        wrote
    };

    let store = open(&file);
    let read = store.get::<T>(path);
    drop(store);

    Trip { wrote, read }
}

fn probe_path() -> StorePath {
    StorePath::from_segments(["probe", "v"])
}

fn trip<T: Serialize + DeserializeOwned>(tag: &str, value: &T) -> Trip<T> {
    trip_at(tag, &probe_path(), value)
}

// ---------------------------------------------------------------------------
// 1. Non-finite floats
// ---------------------------------------------------------------------------

/// msgpack writes the IEEE bits, so every float there is a float here - down to
/// which NaN it was.
#[test]
fn non_finite_floats_survive_bit_for_bit() {
    for (tag, value) in [
        ("nan", f64::NAN),
        ("neg_nan", -f64::NAN),
        ("inf", f64::INFINITY),
        ("neg_inf", f64::NEG_INFINITY),
    ] {
        let back: f64 = trip(&format!("probe_f64_{tag}"), &value)
            .value()
            .unwrap_or_else(|| panic!("{tag}: nothing came back"));
        assert_eq!(
            back.to_bits(),
            value.to_bits(),
            "{tag}: {:#x} came back as {:#x}",
            value.to_bits(),
            back.to_bits()
        );
    }
}

/// A NaN carrying a payload keeps it, which is more than the bit pattern of
/// the canonical one.
#[test]
fn a_nan_payload_survives() {
    let value = f64::from_bits(0x7ff8_0000_dead_beef);
    let back: f64 = trip("probe_nan_payload", &value).value().unwrap();
    assert_eq!(back.to_bits(), value.to_bits());
}

/// f32 as well, and the sign of its NaN.
#[test]
fn f32_non_finite_survives() {
    for (tag, value) in [
        ("nan", f32::NAN),
        ("neg_nan", -f32::NAN),
        ("inf", f32::INFINITY),
        ("neg_inf", f32::NEG_INFINITY),
    ] {
        let back: f32 = trip(&format!("probe_f32_{tag}"), &value).value().unwrap();
        assert_eq!(back.to_bits(), value.to_bits(), "{tag}");
    }
}

// ---------------------------------------------------------------------------
// 2. Numbers
// ---------------------------------------------------------------------------

#[test]
fn integer_extremes_survive() {
    assert_eq!(trip("probe_u64_max", &u64::MAX).value(), Some(u64::MAX));
    assert_eq!(trip("probe_i64_min", &i64::MIN).value(), Some(i64::MIN));
    assert_eq!(trip("probe_i64_max", &i64::MAX).value(), Some(i64::MAX));
    assert_eq!(trip("probe_u64_zero", &0u64).value(), Some(0u64));
    assert_eq!(trip("probe_i8_min", &i8::MIN).value(), Some(i8::MIN));
}

/// `-0.0` compares equal to `0.0`, so only the bits say whether it survived.
#[test]
fn negative_zero_keeps_its_sign() {
    let back: f64 = trip("probe_neg_zero", &-0.0f64).value().unwrap();
    assert_eq!(back.to_bits(), (-0.0f64).to_bits(), "the sign bit went");
}

#[test]
fn float_precision_survives() {
    for (tag, value) in [
        ("tenth", 0.1f64),
        ("sum", 0.1f64 + 0.2f64),
        ("min_positive", f64::MIN_POSITIVE),
        ("subnormal", f64::from_bits(1)),
        ("epsilon", f64::EPSILON),
        ("big", 1.797_693_134_862_315_7e308f64),
    ] {
        let back: f64 = trip(&format!("probe_prec_{tag}"), &value).value().unwrap();
        assert_eq!(back.to_bits(), value.to_bits(), "{tag}");
    }
}

/// A number written at one width and read at another is the store answering a
/// question about a type it was never told. What matters is that it does not
/// answer it wrongly.
#[test]
fn a_width_that_cannot_hold_the_value_is_refused_not_wrapped() {
    let file = TempPath::new("probe_narrowing");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &(-1i64)).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert!(
        store.get::<u64>(&path).is_err(),
        "-1 must not read back as u64::MAX"
    );
    assert_eq!(store.get::<i64>(&path).unwrap(), Some(-1));
    assert_eq!(
        store.get::<i32>(&path).unwrap(),
        Some(-1),
        "a value that fits a narrower type widens without loss"
    );
    drop(store);
}

#[test]
fn a_value_too_large_for_the_type_asked_for_is_refused() {
    let file = TempPath::new("probe_too_large");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &u64::MAX).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert!(store.get::<i64>(&path).is_err(), "u64::MAX is not an i64");
    assert!(store.get::<u32>(&path).is_err(), "u64::MAX is not a u32");
    assert_eq!(store.get::<u64>(&path).unwrap(), Some(u64::MAX));
    drop(store);
}

/// Whether 128-bit integers reach the file at all.
#[test]
fn u128_is_either_kept_or_refused() {
    let value = u128::MAX;
    let Trip { wrote, read } = trip::<u128>("probe_u128", &value);

    match wrote {
        Ok(()) => {
            println!("u128: accepted");
            assert_eq!(
                read.expect("a write that was accepted must read back"),
                Some(value),
                "u128 was written and came back as something else"
            );
        }
        Err(report) => println!("u128: refused at the write - {report:#}"),
    }
}

#[test]
fn i128_is_either_kept_or_refused() {
    let value = i128::MIN;
    let Trip { wrote, read } = trip::<i128>("probe_i128", &value);

    match wrote {
        Ok(()) => {
            println!("i128: accepted");
            assert_eq!(read.expect("accepted writes read back"), Some(value));
        }
        Err(report) => println!("i128: refused at the write - {report:#}"),
    }
}

/// `#[serde(flatten)]` needs a self-describing map, and the store writes
/// structs as arrays. Whichever way it goes, it must not go quietly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Inner {
    b: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Flattened {
    a: u32,
    #[serde(flatten)]
    inner: Inner,
}

#[test]
fn a_flattened_struct_is_either_kept_or_refused() {
    let value = Flattened {
        a: 1,
        inner: Inner { b: 2 },
    };
    let Trip { wrote, read } = trip("probe_flatten", &value);

    match wrote {
        Ok(()) => match read {
            Ok(back) => {
                println!("flatten: accepted, read back {back:?}");
                assert_eq!(back, Some(value), "flatten came back as another value");
            }
            Err(report) => println!("flatten: written and unreadable - {report:#}"),
        },
        Err(report) => println!("flatten: refused at the write - {report:#}"),
    }
}

// ---------------------------------------------------------------------------
// 3. Strings and bytes
// ---------------------------------------------------------------------------

#[test]
fn strings_survive_whatever_they_hold() {
    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("nul", "a\u{0}b".to_string()),
        ("controls", "\u{1}\u{7}\u{8}\u{b}\u{1b}\u{7f}".to_string()),
        ("newlines", "a\r\nb\nc".to_string()),
        ("quotes", "he said \"x\" and 'y'".to_string()),
        ("backslash", "a\\b\\\\c".to_string()),
        ("dots", "a.b..c.".to_string()),
        (
            "unicode",
            "\u{4f60}\u{597d} \u{1f600} \u{5d0}\u{5d1}".to_string(),
        ),
        ("combining", "e\u{301}\u{308}".to_string()),
        ("bom", "\u{feff}x".to_string()),
        ("bidi", "a\u{202e}b\u{202c}c".to_string()),
        ("replacement", "\u{fffd}".to_string()),
        ("max_scalar", "\u{10ffff}".to_string()),
    ];

    for (tag, value) in cases {
        let back = trip(&format!("probe_str_{tag}"), &value).value();
        assert_eq!(back.as_ref(), Some(&value), "{tag}");
    }
}

#[test]
fn a_megabyte_string_survives() {
    let value = "\u{4f60}a".repeat(300_000);
    assert!(value.len() > 1_000_000);
    let back = trip("probe_str_1mb", &value).value();
    assert_eq!(back.as_deref(), Some(value.as_str()));
}

#[test]
fn bytes_survive_and_stay_bytes() {
    let value: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    let back: Vec<u8> = trip("probe_bytes", &value).value().unwrap();
    assert_eq!(back, value);

    let empty: Vec<u8> = Vec::new();
    assert_eq!(trip("probe_bytes_empty", &empty).value(), Some(empty));
}

/// The store forces every `u8` sequence to msgpack's `bin`. A fixed-size array
/// is such a sequence and is read back as a tuple, so this is where the two
/// representations have to agree.
#[test]
fn a_fixed_size_byte_array_round_trips() {
    let value: [u8; 8] = [0, 1, 2, 253, 254, 255, 128, 64];
    let Trip { wrote, read } = trip::<[u8; 8]>("probe_byte_array", &value);
    wrote.expect("the write was refused");
    assert_eq!(read.expect("the read failed"), Some(value));
}

#[test]
fn a_nested_byte_vector_round_trips() {
    let value: Vec<Vec<u8>> = vec![vec![], vec![0], vec![255, 0, 128]];
    assert_eq!(
        trip("probe_bytes_nested", &value).value(),
        Some(value.clone())
    );
}

/// A string is not a byte vector even where the bytes would fit.
#[test]
fn a_string_does_not_read_back_as_bytes_or_the_other_way() {
    let file = TempPath::new("probe_str_vs_bytes");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &"abc".to_string()).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let as_bytes = store.get::<Vec<u8>>(&path);
    // Either answer is defensible; a *different* value is not.
    if let Ok(Some(bytes)) = &as_bytes {
        assert_eq!(bytes.as_slice(), b"abc", "the bytes are not the string's");
    }
    drop(store);
}

// ---------------------------------------------------------------------------
// 4. Path segments and key encoding
// ---------------------------------------------------------------------------

/// One segment holding anything at all is still one segment, and the value at
/// it comes back from the file.
#[test]
fn a_segment_holding_anything_addresses_exactly_itself() {
    let names: Vec<(&str, String)> = vec![
        ("dot", ".".to_string()),
        ("dotdot", "..".to_string()),
        ("dotted", "a.b".to_string()),
        ("trailing_dot", "a.".to_string()),
        ("leading_dot", ".a".to_string()),
        ("backslash", "a\\b".to_string()),
        ("double_backslash", "a\\\\b".to_string()),
        ("trailing_backslash", "a\\".to_string()),
        ("escape_then_dot", "a\\.b".to_string()),
        ("quote", "a\"b".to_string()),
        ("newline", "a\nb".to_string()),
        ("nul", "a\u{0}b".to_string()),
        ("tab", "a\tb".to_string()),
        ("unicode", "\u{4f60}\u{597d}".to_string()),
        ("emoji", "\u{1f600}".to_string()),
        ("init_marker", "__init::probe".to_string()),
        ("table_name", "data".to_string()),
        ("meta_table", "metadata".to_string()),
        ("space", " ".to_string()),
    ];

    for (tag, name) in names {
        let file = TempPath::new(&format!("probe_seg_{tag}"));
        let path = StorePath::from_segments(["probe", &name]);
        let sibling = StorePath::from_segments(["probe", "other"]);

        {
            let store = open(&file);
            store.set(&path, &1u32).unwrap();
            store.set(&sibling, &2u32).unwrap();
            store.flush_prefix(StorePath::root()).unwrap();
        }

        let store = open(&file);
        assert_eq!(
            store.get::<u32>(&path).unwrap(),
            Some(1),
            "{tag}: {name:?} did not survive the reopen"
        );

        let keys = store.scan_keys(StorePath::segment("probe")).unwrap();
        assert_eq!(keys.len(), 2, "{tag}: scan under the parent saw {keys:?}");
        assert!(
            keys.contains(&path),
            "{tag}: the scan did not hand back the path that was written: {keys:?}"
        );
        for key in &keys {
            assert_eq!(key.len(), 2, "{tag}: {key} was split into levels");
        }
        drop(store);
    }
}

/// A level with no name is refused, and refusing it leaves nothing behind.
#[test]
fn an_empty_segment_is_refused_and_writes_nothing() {
    let file = TempPath::new("probe_empty_segment");
    let store = open(&file);

    let refused = store.set(["probe", ""], &1u32);
    assert!(refused.is_err(), "an empty level was accepted");
    assert!(
        refused
            .unwrap_err()
            .downcast_ref::<StorageError>()
            .is_some_and(|e| matches!(e, StorageError::Path)),
        "refused for the wrong reason"
    );

    assert!(
        store.scan_keys(StorePath::root()).unwrap().is_empty(),
        "the refused write left a key behind"
    );
    drop(store);
}

/// A very long name is a key, and redb has to take it whole.
#[test]
fn a_very_long_segment_round_trips() {
    for size in [1_000usize, 100_000, 1_000_000] {
        let file = TempPath::new(&format!("probe_long_seg_{size}"));
        let name = "n".repeat(size);
        let path = StorePath::from_segments(["probe", &name]);

        let wrote = {
            let store = open(&file);
            let wrote = store.set(&path, &7u32);
            let _ = store.flush_prefix(StorePath::root());
            wrote
        };

        let store = open(&file);
        match wrote {
            Ok(()) => assert_eq!(
                store.get::<u32>(&path).unwrap(),
                Some(7),
                "a {size}-byte name was accepted and lost"
            ),
            Err(report) => panic!("a {size}-byte name was refused: {report:?}"),
        }
        drop(store);
    }
}

/// A path of very many levels is one long key, and the levels have to come back
/// out of it.
#[test]
fn a_path_of_many_levels_round_trips() {
    for depth in [16usize, 256, 4_096] {
        let file = TempPath::new(&format!("probe_deep_path_{depth}"));
        let segments: Vec<String> = (0..depth).map(|i| format!("l{i}")).collect();
        let path = StorePath::from_segments(&segments);

        {
            let store = open(&file);
            store.set(&path, &depth).unwrap();
            store.flush_prefix(StorePath::root()).unwrap();
        }

        let store = open(&file);
        assert_eq!(
            store.get::<usize>(&path).unwrap(),
            Some(depth),
            "a {depth}-level path did not come back"
        );
        let keys = store.scan_keys(StorePath::segment("l0")).unwrap();
        assert_eq!(
            keys,
            vec![path.clone()],
            "the scan lost a {depth}-level path"
        );
        assert_eq!(keys[0].len(), depth, "the levels did not survive the key");
        drop(store);
    }
}

/// Two paths that are not the same path must never become the same key.
#[test]
fn no_two_paths_share_a_key() {
    let distinct = vec![
        ("two_levels", StorePath::from_segments(["a", "b"])),
        ("dotted_name", StorePath::from_segments(["a.b"])),
        ("escape_level", StorePath::from_segments(["a\\", "b"])),
        ("escaped_dot_name", StorePath::from_segments(["a\\.b"])),
        (
            "escape_then_dotted",
            StorePath::from_segments(["a\\", ".b"]),
        ),
        ("backslash_name", StorePath::from_segments(["a\\b"])),
        ("three_levels", StorePath::from_segments(["a", "b", "c"])),
    ];

    let file = TempPath::new("probe_key_collision");
    {
        let store = open(&file);
        for (i, (_, path)) in distinct.iter().enumerate() {
            store.set(path, &(i as u32)).unwrap();
        }
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    for (i, (tag, path)) in distinct.iter().enumerate() {
        assert_eq!(
            store.get::<u32>(path).unwrap(),
            Some(i as u32),
            "{tag}: {path} was overwritten by another path"
        );
    }
    assert_eq!(
        store.scan_keys(StorePath::root()).unwrap().len(),
        distinct.len(),
        "two paths landed on one key"
    );
    drop(store);
}

// ---------------------------------------------------------------------------
// 5. Prefix scans
// ---------------------------------------------------------------------------

/// A scan of one subtree must not reach into another whose key merely shares
/// the same opening characters.
#[test]
fn a_scan_stops_at_the_level_boundary() {
    let file = TempPath::new("probe_scan_boundary");

    let under = StorePath::from_segments(["ui", "width"]);
    let deeper = StorePath::from_segments(["ui", "panel", "x"]);
    let at = StorePath::segment("ui");
    let sibling_longer = StorePath::segment("uix");
    let sibling_dashed = StorePath::segment("ui-x");
    let sibling_dotted = StorePath::from_segments(["ui.x"]);
    let sibling_slash = StorePath::segment("ui/x");
    let sibling_before = StorePath::segment("ui!x");
    let sibling_nul = StorePath::segment("ui\u{0}x");
    let sibling_backslash = StorePath::segment("ui\\x");

    let all = vec![
        &under,
        &deeper,
        &at,
        &sibling_longer,
        &sibling_dashed,
        &sibling_dotted,
        &sibling_slash,
        &sibling_before,
        &sibling_nul,
        &sibling_backslash,
    ];

    {
        let store = open(&file);
        for (i, p) in all.iter().enumerate() {
            store.set(*p, &(i as u32)).unwrap();
        }
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let mut seen = store.scan_keys(&at).unwrap();
    seen.sort();
    let mut want = vec![at.clone(), under.clone(), deeper.clone()];
    want.sort();
    assert_eq!(seen, want, "the scan of `ui` crossed a level boundary");

    assert_eq!(
        store.scan_keys(&sibling_longer).unwrap(),
        vec![sibling_longer.clone()]
    );
    assert_eq!(
        store.scan_keys(&sibling_dotted).unwrap(),
        vec![sibling_dotted.clone()]
    );
    assert_eq!(
        store.scan_keys(&sibling_nul).unwrap(),
        vec![sibling_nul.clone()]
    );

    assert_eq!(
        store.scan_keys(StorePath::root()).unwrap().len(),
        all.len(),
        "the root scan lost or invented a key"
    );
    drop(store);
}

/// `delete_prefix` takes the subtree and nothing beside it.
#[test]
fn delete_prefix_stops_at_the_level_boundary() {
    let file = TempPath::new("probe_delete_prefix");

    let at = StorePath::segment("ui");
    let under = StorePath::from_segments(["ui", "width"]);
    let sibling = StorePath::segment("uix");
    let dotted = StorePath::from_segments(["ui.x"]);
    let nul = StorePath::segment("ui\u{0}x");

    {
        let store = open(&file);
        for p in [&at, &under, &sibling, &dotted, &nul] {
            store.set(p, &1u32).unwrap();
        }
        store.flush_prefix(StorePath::root()).unwrap();
        store.delete_prefix(&at).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let mut seen = store.scan_keys(StorePath::root()).unwrap();
    seen.sort();
    let mut want = vec![sibling.clone(), dotted.clone(), nul.clone()];
    want.sort();
    assert_eq!(seen, want, "delete_prefix reached past its subtree");
    drop(store);
}

/// The escape character is the other half of the key encoding, and a name that
/// ends with one puts it right where the separator that bounds a subtree goes.
#[test]
fn a_name_ending_in_an_escape_does_not_widen_its_subtree() {
    let file = TempPath::new("probe_escape_boundary");

    let at = StorePath::segment("a\\");
    let under = StorePath::from_segments(["a\\", "x"]);
    let dotted = StorePath::from_segments(["a\\.x"]);
    let plain = StorePath::segment("a");
    let plain_under = StorePath::from_segments(["a", "x"]);
    let two_escapes = StorePath::segment("a\\\\");

    let all = vec![&at, &under, &dotted, &plain, &plain_under, &two_escapes];

    {
        let store = open(&file);
        for (i, p) in all.iter().enumerate() {
            store.set(*p, &(i as u32)).unwrap();
        }
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    for (i, p) in all.iter().enumerate() {
        assert_eq!(store.get::<u32>(*p).unwrap(), Some(i as u32), "{p}");
    }

    let mut seen = store.scan_keys(&at).unwrap();
    seen.sort();
    let mut want = vec![at.clone(), under.clone()];
    want.sort();
    assert_eq!(
        seen, want,
        "the scan of a name ending in an escape reached out"
    );

    let mut seen = store.scan_keys(&plain).unwrap();
    seen.sort();
    let mut want = vec![plain.clone(), plain_under.clone()];
    want.sort();
    assert_eq!(seen, want, "`a` reached into `a\\`");

    assert_eq!(store.scan_keys(StorePath::root()).unwrap().len(), all.len());
    drop(store);
}

/// The root is a path. Whether a value may live at it, and what a scan says
/// about it if one does.
#[test]
fn a_value_at_the_root_is_either_refused_or_addressable() {
    let file = TempPath::new("probe_root_value");

    let wrote = {
        let store = open(&file);
        let wrote = store.set(StorePath::root(), &7u32);
        store.set(["a"], &1u32).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        wrote
    };

    let store = open(&file);
    match wrote {
        Err(report) => println!("a value at the root: refused - {report:#}"),
        Ok(()) => {
            assert_eq!(
                store.get::<u32>(StorePath::root()).unwrap(),
                Some(7),
                "the root took a value and did not give it back"
            );
            let keys = store.scan_keys(StorePath::root()).unwrap();
            println!("a value at the root: kept, root scan = {keys:?}");
            assert!(
                keys.contains(&StorePath::root()),
                "the value at the root is not in a scan of the root: {keys:?}"
            );
            assert_eq!(
                store.scan_keys(["a"]).unwrap(),
                vec![StorePath::segment("a")],
                "the value at the root leaked into another subtree"
            );
        }
    }
    drop(store);
}

/// A path that is a prefix of another holds its own value, and neither write
/// erases the other.
#[test]
fn a_prefix_and_what_is_under_it_both_survive_a_reopen() {
    let file = TempPath::new("probe_prefix_and_child");
    let node = StorePath::from_segments(["a", "b"]);
    let child = StorePath::from_segments(["a", "b", "c"]);
    let grandchild = StorePath::from_segments(["a", "b", "c", "d"]);

    {
        let store = open(&file);
        store.set(&node, &1u32).unwrap();
        store.set(&child, &2u32).unwrap();
        store.set(&grandchild, &3u32).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert_eq!(store.get::<u32>(&node).unwrap(), Some(1));
    assert_eq!(store.get::<u32>(&child).unwrap(), Some(2));
    assert_eq!(store.get::<u32>(&grandchild).unwrap(), Some(3));
    assert_eq!(store.scan_keys(&node).unwrap().len(), 3);
    assert_eq!(store.scan_keys(&child).unwrap().len(), 2);
    drop(store);
}

/// Nothing a scan hands back was invented, and nothing the store wrote for
/// itself shows up as data.
#[test]
fn the_stores_own_bookkeeping_is_not_readable_as_a_path() {
    let file = TempPath::new("probe_residue");

    {
        let store = open(&file);
        store.set(["probe", "a"], &1u32).unwrap();
        store.mark_initialized(&ns("probe")).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert_eq!(
        store.scan_keys(StorePath::root()).unwrap(),
        vec![StorePath::from_segments(["probe", "a"])],
        "something other than the written path is readable"
    );
    assert!(store.is_initialized(&ns("probe")).unwrap());
    assert!(
        store
            .get::<serde_json::Value>(StorePath::segment("__init::probe"))
            .unwrap()
            .is_none(),
        "the initialization marker is readable as data"
    );
    drop(store);
}

/// A write and then a delete leaves the store exactly as it was.
#[test]
fn a_delete_leaves_no_residue_across_a_reopen() {
    let file = TempPath::new("probe_delete_residue");
    let path = StorePath::from_segments(["probe", "gone"]);

    {
        let store = open(&file);
        store.set(&path, &1u32).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        store.delete(&path).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert_eq!(store.get::<u32>(&path).unwrap(), None);
    assert!(store.scan_keys(StorePath::root()).unwrap().is_empty());
    drop(store);
}

// ---------------------------------------------------------------------------
// 6. Structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Unit;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EmptyBraces {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Newtype(u32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Tuple(u32, String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Fields {
    a: u32,
    b: String,
    c: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Shape {
    Nothing,
    One(u32),
    Two(u32, u32),
    Named { x: u32, y: String },
}

#[test]
fn empty_and_degenerate_structures_survive() {
    assert_eq!(trip("probe_unit_struct", &Unit).value(), Some(Unit));
    assert_eq!(
        trip("probe_empty_braces", &EmptyBraces {}).value(),
        Some(EmptyBraces {})
    );
    assert_eq!(trip("probe_unit", &()).value(), Some(()));
    assert_eq!(trip("probe_newtype", &Newtype(9)).value(), Some(Newtype(9)));
    assert_eq!(
        trip("probe_tuple_struct", &Tuple(1, "x".into())).value(),
        Some(Tuple(1, "x".into()))
    );

    let empty_vec: Vec<u32> = Vec::new();
    assert_eq!(trip("probe_empty_vec", &empty_vec).value(), Some(empty_vec));

    let empty_map: BTreeMap<String, u32> = BTreeMap::new();
    assert_eq!(trip("probe_empty_map", &empty_map).value(), Some(empty_map));

    let empty_string_key: BTreeMap<String, u32> = [(String::new(), 1u32)].into_iter().collect();
    assert_eq!(
        trip("probe_empty_map_key", &empty_string_key).value(),
        Some(empty_string_key)
    );
}

#[test]
fn every_enum_shape_survives() {
    for (tag, value) in [
        ("unit", Shape::Nothing),
        ("newtype", Shape::One(3)),
        ("tuple", Shape::Two(3, 4)),
        (
            "struct",
            Shape::Named {
                x: 5,
                y: "z".into(),
            },
        ),
    ] {
        let back = trip(&format!("probe_enum_{tag}"), &value).value();
        assert_eq!(back.as_ref(), Some(&value), "{tag}");
    }
}

#[test]
fn option_none_survives() {
    let none: Option<u32> = None;
    assert_eq!(trip("probe_none", &none).value(), Some(none));

    let some: Option<u32> = Some(0);
    assert_eq!(trip("probe_some_zero", &some).value(), Some(some));

    let fields = Fields {
        a: 0,
        b: String::new(),
        c: None,
    };
    assert_eq!(trip("probe_fields_none", &fields).value(), Some(fields));
}

/// `Some(None)` and `None` are two values, and msgpack spells both `nil`.
///
/// Both reach the file as the single byte `c0` and both read back as `None`,
/// so the write is refused rather than answering `Ok` for a value the store
/// cannot return. It is serde's representation and not msgpack's doing: the
/// outer `Some` has nothing of its own to write, and JSON does the same with
/// `null`. ron is the one engine that spells the `Option` out.
///
/// The layers carrying a value are untouched: `Some(Some(1))` goes and comes
/// back whole, and `None` is written as itself.
#[test]
fn nested_option_is_refused_where_its_outer_layer_would_be_lost() {
    let inner_none: Option<Option<u32>> = Some(None);
    let outer_none: Option<Option<u32>> = None;

    let refused = trip("probe_opt_some_none", &inner_none)
        .wrote
        .expect_err("Some(None) was taken, and it reads back as None");
    assert!(
        format!("{refused:?}").contains("holding nothing"),
        "{refused:?}"
    );

    assert_eq!(
        trip("probe_opt_none", &outer_none).value(),
        Some(outer_none)
    );
    assert_eq!(
        trip("probe_opt_some_some", &Some(Some(1u32))).value(),
        Some(Some(Some(1u32))),
        "a layer carrying a value survives"
    );
}

/// A unit inside an option is the same question with a different inner type,
/// and it has the same answer: `Some(())` comes back as `None`.
#[test]
fn an_option_of_unit_loses_its_layer() {
    let some_unit: Option<()> = Some(());
    assert_eq!(
        trip("probe_opt_unit", &some_unit).value(),
        Some(None),
        "Some(()) is kept today, which would be the repair"
    );
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct HoldsNested {
    before: u32,
    maybe: Option<Option<u32>>,
    after: u32,
}

/// The refusal follows the value wherever it is nested.
///
/// A struct field, a collection element and a map entry each cost the whole
/// write, because a store writes one value at one path and there is no half of
/// a struct to keep. The fields around it go too, which is the price of not
/// storing a value that comes back as a different one.
#[test]
fn a_nested_option_inside_a_structure_costs_the_whole_write() {
    let held = HoldsNested {
        before: 1,
        maybe: Some(None),
        after: 2,
    };
    let in_a_struct = trip("probe_nested_in_struct", &held)
        .wrote
        .expect_err("the struct was taken, and its middle field reads back as None");
    assert!(format!("{in_a_struct:?}").contains("holding nothing"));

    let list: Vec<Option<Option<u32>>> = vec![Some(Some(1)), Some(None), None];
    let in_a_list = trip("probe_nested_in_vec", &list)
        .wrote
        .expect_err("the list was taken, and its second and third entries became one");
    assert!(format!("{in_a_list:?}").contains("holding nothing"));

    let map: BTreeMap<String, Option<Option<u32>>> =
        [("a".to_string(), Some(None)), ("b".to_string(), None)]
            .into_iter()
            .collect();
    let in_a_map = trip("probe_nested_in_map", &map)
        .wrote
        .expect_err("the map was taken, and its two entries became one");
    assert!(format!("{in_a_map:?}").contains("holding nothing"));
}

/// A field skipped on the way out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Skipping {
    first: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    middle: Option<u32>,
    last: String,
}

/// A field that is not written does not move the ones after it.
///
/// The store used to write a struct as an array, so the fields were read back
/// by position and `skip_serializing_if` removed an element from the middle -
/// everything after it moved up one place. The shift was caught here only
/// because a `String` will not decode where an `Option<u32>` is expected: the
/// write returned `Ok`, the flush landed, and the read failed, so the value was
/// on disk and could not be got back. `#[serde(default)]` did not help, because
/// nothing was missing as far as the reader could tell - the array was simply
/// one shorter than the struct.
///
/// With the fields written by name there is no position to shift.
#[test]
fn a_skipped_field_shifts_the_ones_after_it() {
    let value = Skipping {
        first: 1,
        middle: None,
        last: "tail".into(),
    };
    assert_eq!(
        trip("probe_skip_field", &value).value(),
        Some(value),
        "a skipped field still moved the ones after it"
    );

    let filled = Skipping {
        first: 1,
        middle: Some(9),
        last: "tail".into(),
    };
    assert_eq!(
        trip("probe_skip_field_filled", &filled).value(),
        Some(filled),
        "with nothing skipped the same struct round-trips"
    );
}

/// The same shift where the types line up, which is where nothing catches it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SkippingAligned {
    first: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    middle: Option<u32>,
    #[serde(default)]
    last: u32,
}

/// The same shift where the types line up, which is where nothing caught it.
///
/// **This was the silent one.** Skipping the middle field moved `last` into
/// `middle` and gave `last` its default: the write returned `Ok`, the read
/// returned `Ok`, and the value was not the one that had been written. Named
/// fields end it - `last` is found by its name whether or not `middle` was
/// written.
#[test]
fn a_skipped_field_silently_moves_the_next_one_when_the_types_agree() {
    let value = SkippingAligned {
        first: 1,
        middle: None,
        last: 99,
    };
    assert_eq!(
        trip("probe_skip_aligned", &value).value(),
        Some(value),
        "a skipped field still moved the next one into its place"
    );
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SizeV1 {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SizeV2 {
    height: u32,
    width: u32,
}

/// **This was the silent alteration most likely to be met by accident.** A
/// struct was stored as an array, so its fields were addressed by position and
/// never by name: swapping two fields of the same type in the source swapped
/// every value already on disk, and renaming one was invisible. Nothing in the
/// file recorded which name held which slot, so no read could notice - while
/// the text engines caught the same edit, because json and toml store names.
///
/// They are stored by name here too now.
#[test]
fn a_struct_is_stored_by_position_so_reordering_its_fields_swaps_the_values() {
    let file = TempPath::new("probe_field_order");
    let path = probe_path();

    {
        let store = open(&file);
        store
            .set(
                &path,
                &SizeV1 {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert_eq!(
        store.get::<SizeV2>(&path).unwrap(),
        Some(SizeV2 {
            width: 1280,
            height: 720,
        }),
        "reordering two same-typed fields still swaps what they hold"
    );
    drop(store);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum PickV1 {
    Alpha(u32),
    Beta(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum PickV2 {
    Beta(u32),
    Alpha(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum PickRenamed {
    Gamma(u32),
    Beta(u32),
}

/// An enum variant is stored by name, unlike a struct field, which is stored by
/// position. So reordering the variants changes nothing and renaming one is
/// caught - the opposite of what a struct does, in the same file, in the same
/// codec.
#[test]
fn an_enum_is_stored_by_name_where_a_struct_field_is_stored_by_position() {
    let file = TempPath::new("probe_variant_order");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &PickV1::Alpha(7)).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert_eq!(
        store.get::<PickV2>(&path).unwrap(),
        Some(PickV2::Alpha(7)),
        "reordering the variants moved the value to another one"
    );
    assert!(
        store.get::<PickRenamed>(&path).is_err(),
        "a renamed variant decoded as one of the ones that are left"
    );
    drop(store);
}

/// A map whose keys are not strings, which no text engine can hold as one.
#[test]
fn a_map_with_non_string_keys_survives() {
    let value: BTreeMap<u64, String> = [
        (0u64, "zero".to_string()),
        (u64::MAX, "max".to_string()),
        (7, "seven".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(trip("probe_int_keys", &value).value(), Some(value.clone()));

    let bytes: BTreeMap<Vec<u8>, u32> = [(vec![0u8, 1, 2], 1u32), (vec![], 2u32)]
        .into_iter()
        .collect();
    let Trip { wrote, read } = trip("probe_byte_keys", &bytes);
    wrote.expect("the write was refused");
    assert_eq!(
        read.expect("byte-keyed map would not read back"),
        Some(bytes)
    );
}

/// A map key holding the separator, an escape or a NUL is a value, not a path,
/// and nothing should escape it.
#[test]
fn a_value_map_key_holding_a_separator_survives() {
    let value: BTreeMap<String, u32> = [
        ("a.b".to_string(), 1u32),
        ("a\\b".to_string(), 2),
        ("a\u{0}b".to_string(), 3),
        (String::new(), 4),
        ("\u{1f600}".to_string(), 5),
    ]
    .into_iter()
    .collect();
    assert_eq!(trip("probe_map_keys", &value).value(), Some(value.clone()));
}

#[test]
fn a_char_survives() {
    for (tag, value) in [
        ("ascii", 'a'),
        ("nul", '\u{0}'),
        ("max", '\u{10ffff}'),
        ("emoji", '\u{1f600}'),
    ] {
        assert_eq!(
            trip(&format!("probe_char_{tag}"), &value).value(),
            Some(value),
            "{tag}"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Depth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Nest {
    End,
    More(Box<Nest>),
}

fn nest(depth: usize) -> Nest {
    let mut n = Nest::End;
    for _ in 0..depth {
        n = Nest::More(Box::new(n));
    }
    n
}

const DEPTH_CHILD: &str = "AME_PROBE_REDB_DEPTH";
const DEPTH_FILE: &str = "AME_PROBE_REDB_FILE";

/// The stack the depth probe runs on, named rather than inherited so the
/// numbers it reports mean something.
const PROBE_STACK: usize = 8 * 1024 * 1024;

/// Builds and drops the value, touching no store. The subtraction that says
/// whether a depth limit belongs to the codec or to the type itself, whose
/// `Drop` recurses just as the serializer does.
fn depth_plain(depth: usize) {
    let value = nest(depth);
    std::hint::black_box(&value);
}

/// The write half alone: serialize into the store and commit, never reading it
/// back.
fn depth_write_only(depth: usize) {
    let file = TempPath::new(&format!("probe_nest_w_{depth}"));
    let value = nest(depth);
    let store = open(&file);
    store.set(["probe", "nest"], &value).unwrap();
    store.flush_prefix(StorePath::root()).unwrap();
    drop(store);
}

fn depth_store(depth: usize) {
    let file = TempPath::new(&format!("probe_nest_{depth}"));
    let value = nest(depth);
    {
        let store = open(&file);
        store.set(["probe", "nest"], &value).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }
    let store = open(&file);
    let back = store.get::<Nest>(["probe", "nest"]).unwrap();
    assert_eq!(back, Some(value));
    drop(store);
}

/// Writes a nested value into a named file and commits it, so another process
/// can be the one that tries to read it.
fn depth_write_into(file: &Path, depth: usize) {
    let store = StoreBuilder::new(file)
        .backend(Backend::Redb)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .expect("the store opened");
    store.set(["probe", "nest"], &nest(depth)).unwrap();
    store.flush_prefix(StorePath::root()).unwrap();
    drop(store);
}

fn depth_read_from(file: &Path) {
    let store = StoreBuilder::new(file)
        .backend(Backend::Redb)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .expect("the store opened");
    let back = store.get::<Nest>(["probe", "nest"]);
    println!("read back: {}", if back.is_ok() { "ok" } else { "error" });
    drop(store);
}

fn depth_child_at(mode: &str, depth: usize, file: Option<&Path>) -> (bool, String) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("msgpack_nesting_boundary")
        .env(DEPTH_CHILD, format!("{mode}:{depth}"));
    if let Some(file) = file {
        command.env(DEPTH_FILE, file);
    }
    let out = command.output().expect("spawning the child failed");

    let why = String::from_utf8_lossy(&out.stderr)
        .lines()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .join(" | ");
    (out.status.success(), why)
}

fn depth_child(mode: &str, depth: usize) -> (bool, String) {
    depth_child_at(mode, depth, None)
}

fn depth_child_succeeds(mode: &str, depth: usize) -> bool {
    depth_child(mode, depth).0
}

/// The largest depth that survives, by halving between one that does and one
/// that does not.
fn deepest_that_survives(mode: &str, ceiling: usize) -> usize {
    if depth_child_succeeds(mode, ceiling) {
        return ceiling;
    }

    let (mut good, mut bad) = (0usize, ceiling);
    while bad - good > 1 {
        let mid = good + (bad - good) / 2;
        if depth_child_succeeds(mode, mid) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    good
}

/// Where a nested value stops round-tripping, found in a child process so a
/// stack that runs out takes only the child with it.
///
/// `serde_json` refuses past 128 by design, with an error. msgpack has no such
/// limit written down, and rmp_serde counts nothing: the recursion runs until
/// the stack does, which is why the store imposes a ceiling of its own.
///
/// That ceiling is what the two halves now stop at together, and the value of
/// this probe is that they do. A write reaching deeper than a later read is a
/// value committed and lost - the shape the depth budget exists to prevent -
/// and this is where it would show up again.
#[test]
fn msgpack_nesting_boundary() {
    if let Ok(spec) = std::env::var(DEPTH_CHILD) {
        let (mode, depth) = spec.split_once(':').expect("mode:depth");
        let depth: usize = depth.parse().unwrap();
        let mode = mode.to_string();

        std::thread::Builder::new()
            .stack_size(PROBE_STACK)
            .spawn(move || {
                let named = || {
                    std::path::PathBuf::from(
                        std::env::var(DEPTH_FILE).expect("the child was given no file"),
                    )
                };
                match mode.as_str() {
                    "plain" => depth_plain(depth),
                    "write" => depth_write_only(depth),
                    "store" => depth_store(depth),
                    "write_at" => depth_write_into(&named(), depth),
                    "read_at" => depth_read_from(&named()),
                    other => panic!("unknown mode {other}"),
                }
            })
            .unwrap()
            .join()
            .expect("the probe thread died");

        std::process::exit(0);
    }

    let ceiling = 1_000_000usize;
    let plain = deepest_that_survives("plain", ceiling);
    let write = deepest_that_survives("write", ceiling);
    let store = deepest_that_survives("store", ceiling);
    let (_, over) = depth_child("store", store + 1);

    println!("nesting on an {PROBE_STACK}-byte stack, debug build:");
    println!("  building and dropping the value alone: {plain}");
    println!("  written and flushed:                   {write}");
    println!("  written, flushed, reopened and read:   {store}");
    println!("  one deeper than that: {over}");

    assert!(
        store >= 128,
        "msgpack gave out at or before json's documented limit of 128"
    );
    assert!(
        store <= plain,
        "the codec carried a value the type itself could not be dropped at"
    );

    assert_eq!(
        write, store,
        "a depth the write takes and a later read cannot is a value committed and lost"
    );

    let file = TempPath::new("probe_nest_unreadable");
    let (wrote, why) = depth_child_at("write_at", store + 1, Some(file.path()));

    println!("  writing one deeper: {wrote} - {why}");
    assert!(!wrote, "a value past the ceiling was taken");
}

// ---------------------------------------------------------------------------
// 8. Large values and many keys
// ---------------------------------------------------------------------------

#[test]
fn a_multi_megabyte_value_round_trips() {
    let value: Vec<u8> = (0..8u32 * 1024 * 1024).map(|i| i as u8).collect();
    let file = TempPath::new("probe_big_value");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &value).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let back = store.get::<Vec<u8>>(&path).unwrap().expect("nothing back");
    assert_eq!(back.len(), value.len());
    assert_eq!(back, value);
    drop(store);
}

#[test]
fn a_map_of_many_thousands_of_entries_round_trips() {
    let value: BTreeMap<String, u64> = (0..100_000u64).map(|i| (format!("k{i}"), i)).collect();
    let file = TempPath::new("probe_big_map");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &value).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let back = store
        .get::<BTreeMap<String, u64>>(&path)
        .unwrap()
        .expect("nothing back");
    assert_eq!(back.len(), value.len());
    assert_eq!(back, value);
    drop(store);
}

#[test]
fn many_keys_all_come_back() {
    let file = TempPath::new("probe_many_keys");
    let count = 20_000u32;

    {
        let store = open(&file);
        for i in 0..count {
            store.set(["probe", &format!("k{i}")], &i).unwrap();
        }
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let keys = store.scan_keys(StorePath::segment("probe")).unwrap();
    assert_eq!(keys.len() as u32, count, "the reopen lost or invented keys");
    assert_eq!(store.get::<u32>(["probe", "k19999"]).unwrap(), Some(19_999));
    drop(store);
}

// ---------------------------------------------------------------------------
// Beyond the list
// ---------------------------------------------------------------------------

/// The same path written twice keeps the second value, across the flush that
/// separates them and the one that does not.
#[test]
fn the_last_write_is_the_one_that_reads_back() {
    let file = TempPath::new("probe_last_write");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &1u32).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        store.set(&path, &2u32).unwrap();
        store.set(&path, &3u32).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert_eq!(store.get::<u32>(&path).unwrap(), Some(3));
    drop(store);
}

/// A write that never got a flush is still what a read of the same store sees,
/// and a `save_now` is what makes it what the next process sees.
#[test]
fn an_unflushed_write_is_visible_to_the_store_and_lost_without_a_flush() {
    let file = TempPath::new("probe_unflushed");
    let path = probe_path();

    {
        let store = open(&file);
        store.set(&path, &42u32).unwrap();
        assert_eq!(
            store.get::<u32>(&path).unwrap(),
            Some(42),
            "a buffered write is not visible to its own store"
        );
        store.save_now().unwrap();
    }

    let store = open(&file);
    assert_eq!(
        store.get::<u32>(&path).unwrap(),
        Some(42),
        "save_now did not commit the buffer"
    );
    drop(store);
}

/// Bytes that will not decode as the type asked for are a failure, not an
/// absence: the difference between "nothing is stored here" and "something is,
/// and it is not what you said" is the whole of what a caller can act on.
#[test]
fn a_type_mismatch_is_a_failure_not_an_absence() {
    let file = TempPath::new("probe_mismatch");
    let path = probe_path();

    {
        let store = open(&file);
        store
            .set(
                &path,
                &Fields {
                    a: 1,
                    b: "x".into(),
                    c: None,
                },
            )
            .unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    let wrong = store.get::<u32>(&path);
    assert!(wrong.is_err(), "a struct read back as a u32: {wrong:?}");
    let report = wrong.unwrap_err();
    assert_eq!(
        report.current_context(),
        &StorageError::Read,
        "the outermost context names what the caller asked for: {report:?}"
    );
    assert!(
        report.contains::<amethystate::errors::CodecError>(),
        "the codec's refusal is the cause underneath: {report:?}"
    );
    drop(store);
}

/// A value written under a namespace flag and read after a reopen sees both,
/// and the flag is not a key.
#[test]
fn a_namespace_flag_and_its_data_land_together() {
    let file = TempPath::new("probe_namespace");

    {
        let store = open(&file);
        store.set(["ns", "a"], &1u32).unwrap();
        store.mark_initialized(&ns("ns")).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert!(store.is_initialized(&ns("ns")).unwrap());
    assert_eq!(store.get::<u32>(["ns", "a"]).unwrap(), Some(1));
    assert_eq!(store.scan_keys(StorePath::segment("ns")).unwrap().len(), 1);
    drop(store);
}

/// A namespace whose name holds the separator has its own flag, not one shared
/// with the levels it looks like.
#[test]
fn a_namespace_name_holding_a_separator_has_its_own_flag() {
    let file = TempPath::new("probe_namespace_dotted");

    {
        let store = open(&file);
        store.mark_initialized(&ns("a.b")).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(&file);
    assert!(store.is_initialized(&ns("a.b")).unwrap());
    assert!(
        !store.is_initialized(&ns("a")).unwrap(),
        "marking `a.b` marked `a` as well"
    );
    assert!(!store.is_initialized(&ns("a\\.b")).unwrap());
    drop(store);
}

/// A `HashMap` has no order and msgpack keeps whatever order it was given;
/// what has to survive is the set of entries, not the order.
#[test]
fn a_hash_map_survives_as_a_set_of_entries() {
    let value: HashMap<String, u32> = (0..1_000u32).map(|i| (format!("k{i}"), i)).collect();
    let back: HashMap<String, u32> = trip("probe_hashmap", &value).value().unwrap();
    assert_eq!(back, value);
}

/// The bytes a scan hands back are the bytes a later scan hands back: a write
/// followed by a delete leaves the store byte-identical, not merely
/// value-identical.
#[test]
fn a_write_and_a_delete_leave_the_bytes_as_they_were() {
    let file = TempPath::new("probe_bytes_stable");
    let keep = StorePath::from_segments(["probe", "keep"]);
    let gone = StorePath::from_segments(["probe", "gone"]);

    let before = {
        let store = open(&file);
        store.set(&keep, &"value".to_string()).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        store.scan_prefix(StorePath::root()).unwrap()
    };

    let after = {
        let store = open(&file);
        store.set(&gone, &"other".to_string()).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        store.delete(&gone).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        store.scan_prefix(StorePath::root()).unwrap()
    };

    assert_eq!(before, after, "the round trip changed the stored bytes");
}

// ---------------------------------------------------------------------------
// The migration adapter's own scan
// ---------------------------------------------------------------------------

/// Two maps in one struct, one of whose names starts the other's.
///
/// `routes` and `routes_v2` are two levels, and every scan in the store proper
/// says so: `subtree_bound` puts a separator after the prefix, so `routes` does
/// not reach `routes_v2`. The migration adapter has its own scan, and that one
/// compares characters - `key.starts_with(prefix)` with nothing after it.
mod migration_probe {
    use amethystate::{AmeData, ReactiveMap, migrate};
    use amethystate_macros::amethystate;

    pub mod v1 {
        use super::*;

        #[amethystate(prefix = "probe_mig", version = 1)]
        pub struct Routing {
            pub routes: ReactiveMap<String, String>,
            pub routes_v2: ReactiveMap<String, String>,
        }
    }

    #[amethystate(prefix = "probe_mig", version = 2)]
    pub struct Routing {
        pub routes: ReactiveMap<String, String>,
        pub routes_v2: ReactiveMap<String, String>,
        #[amestate(default = 0u32)]
        pub generation: u32,
    }

    #[migrate]
    fn probe_routing_v1_to_v2(
        old: AmeData<v1::Routing>,
        _ctx: &mut amethystate::migration::MigrationContext,
    ) -> amethystate::MigrationResult<AmeData<Routing>> {
        Ok(AmeData::<Routing> {
            routes: old.routes,
            routes_v2: old.routes_v2,
            generation: 1,
        })
    }
}

/// The metadata table holds two kinds of row under one key space: a
/// component's `PrefixMeta` at its prefix, and a namespace's initialization
/// flag at `__init::` followed by the namespace. The namespace of a component
/// *is* its prefix, so a component whose prefix is `__init::` followed by
/// another component's prefix addresses the other one's flag.
mod meta_collision_probe {
    use amethystate_macros::amethystate;

    #[amethystate(prefix = "probe_meta", version = 1)]
    pub struct Plain {
        #[amestate(default = 1u32)]
        pub n: u32,
    }

    #[amethystate(prefix = "__init::probe_meta", version = 1)]
    pub struct Shadow {
        #[amestate(default = 2u32)]
        pub n: u32,
    }
}

/// Whether the two kinds of metadata row can be made to land on one key.
#[test]
fn a_prefix_that_spells_another_prefixs_init_flag() {
    use meta_collision_probe::{Plain, Shadow};

    let file = TempPath::new("probe_meta_collision");

    {
        let store = open(&file);
        let shadow = Shadow::new_with(&store).unwrap();
        shadow.n().set(20).unwrap();
        let plain = Plain::new_with(&store).unwrap();
        plain.n().set(10).unwrap();
        store.save_now().unwrap();
    }

    let reopened = StoreBuilder::new(file.path())
        .backend(Backend::Redb)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build_with_migration();

    match reopened {
        Err(report) => panic!("the store no longer opens: {report:#}"),
        Ok((store, report)) => {
            println!(
                "meta collision: failures = {}, report = {report:?}",
                report.has_failures()
            );
            let plain = Plain::new_with(&store).unwrap();
            let shadow = Shadow::new_with(&store).unwrap();
            assert_eq!(plain.n().get(), 10, "the value under `probe_meta` changed");
            assert_eq!(
                shadow.n().get(),
                20,
                "the value under `__init::probe_meta` changed"
            );
            assert!(
                !report.has_failures(),
                "the two metadata rows collided: {report:?}"
            );
            drop(store);
        }
    }
}

/// Nothing here is written wrongly and no name is unusual: two sibling maps,
/// one named `routes` and one named `routes_v2`. Every write returns `Ok`, and
/// the store's own scans are right - `scan_keys(["probe_mig", "routes"])` sees
/// one entry, because `subtree_bound` puts a separator after the prefix.
///
/// The migration adapter has a second scan of its own, and that one compares
/// characters: `key.starts_with(prefix)`. So loading the `routes` map for a
/// migration picks up `probe_mig.routes_v2.b`, and `scan_map` then refuses the
/// key it was just handed as not being under the map it scanned.
///
/// The failure is not the loud kind. The store opens, the component is left at
/// its old version with the failure recorded in the report, and construction of
/// the new struct succeeds - so an application that does not read the report
/// runs its new code over data that was never migrated, and every later open
/// fails the same way. The value written to `routes_v2` is what makes the
/// migration of `routes` impossible, and nothing at the write said so.
#[test]
fn a_migration_scans_past_the_level_boundary_and_then_refuses_what_it_found() {
    use migration_probe::{Routing, v1};

    let file = TempPath::new("probe_mig_boundary");

    {
        let store = open(&file);
        let cfg = v1::Routing::new_with(&store).unwrap();
        cfg.routes().insert("a".into(), &"one".into()).unwrap();
        cfg.routes_v2().insert("b".into(), &"two".into()).unwrap();
        store.save_now().unwrap();

        assert_eq!(
            store.scan_keys(["probe_mig", "routes"]).unwrap().len(),
            1,
            "the store's own scan already stops at the level boundary"
        );
    }

    let (store, report) = StoreBuilder::new(file.path())
        .backend(Backend::Redb)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build_with_migration()
        .expect("the file still opens");

    // This probe recorded a defect and now records its absence. The adapter
    // filtered its scan with `key.starts_with(prefix)`, so loading the map
    // named `routes` picked up `routes_v2`'s entries and then refused them for
    // not being under the map they were scanned from - a migration that failed
    // for good, leaving the component stuck at v1 while the new struct ran on
    // unmigrated data. It uses `utils::is_under` now, as the store's own scans
    // always did.
    assert!(
        !report.has_failures(),
        "a map whose name begins another map's name still breaks the migration: {report:?}"
    );

    let cfg = Routing::new_with(&store).unwrap();
    assert_eq!(
        cfg.generation().get(),
        1,
        "the migration did not run, or ran without writing"
    );
    assert_eq!(cfg.routes().keys().collect::<Vec<_>>(), vec![
        "a".to_string()
    ]);
    assert_eq!(
        cfg.routes_v2().keys().collect::<Vec<_>>(),
        vec!["b".to_string()],
        "the sibling map was taken along by the scan"
    );
    drop(store);
}
