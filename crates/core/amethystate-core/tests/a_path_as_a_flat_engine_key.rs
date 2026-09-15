use amethystate_core::path::{Key, StorePath, StorePathError};

fn key(levels: &[&str]) -> Key {
    StorePath::from_segments(levels.iter().copied()).key()
}

fn levels(key: &Key) -> Vec<String> {
    key.path()
        .expect("the key spells a path")
        .segments()
        .map(|level| level.as_str().to_string())
        .collect()
}

#[test]
fn every_path_reads_back_as_itself() {
    for path in [
        vec!["ui"],
        vec!["ui", "theme"],
        vec!["ui", "theme", "dark"],
        vec!["a.b"],
        vec!["a", "b"],
        vec!["a\\b"],
        vec!["with space", "and.dot", "and\\escape"],
        vec!["\u{1F600}", "\u{4e2d}\u{6587}"],
        vec!["\0"],
        vec!["a\0b", "c"],
        vec!["\0\0", "\0"],
        vec!["\u{1}"],
        vec!["\0\u{1}", "\u{1}\0"],
    ] {
        let held = key(&path);
        assert_eq!(levels(&held), path, "{path:?} did not read back");
    }
}

#[test]
fn the_root_is_no_bytes_and_reads_back_as_the_root() {
    let root = StorePath::root().key();

    assert!(root.is_root());
    assert_eq!(root.as_bytes(), b"");
    assert!(root.path().expect("the root spells a path").is_root());
    assert_eq!(root, Key::root());
}

#[test]
fn byte_order_is_the_order_of_the_levels() {
    let mut paths: Vec<Vec<&str>> = vec![
        vec!["pot"],
        vec!["pot", "ato"],
        vec!["pot!luck"],
        vec!["potato"],
        vec!["a"],
        vec!["a", "b"],
        vec!["a", "b", "c"],
        vec!["a", "c"],
        vec!["ab"],
        vec!["a!"],
        vec!["b"],
        vec!["a\0"],
        vec!["a\u{1}"],
        vec!["a\u{2}"],
    ];

    paths.sort();

    let mut by_key = paths.clone();
    by_key.sort_by_key(|path| key(path));

    assert_eq!(by_key, paths);
}

#[test]
fn a_subtree_is_a_byte_prefix_and_nothing_else() {
    let under = key(&["pot"]);

    assert!(key(&["pot"]).under(&under));
    assert!(key(&["pot", "ato"]).under(&under));
    assert!(key(&["pot", "ato", "es"]).under(&under));

    assert!(!key(&["potato"]).under(&under));
    assert!(!key(&["pot!luck"]).under(&under));
    assert!(!key(&["po"]).under(&under));
    assert!(!key(&["pots"]).under(&under));
}

#[test]
fn a_name_holding_the_bytes_the_encoding_uses_is_not_under_the_prefix() {
    let under = key(&["ui"]);

    for beside in [
        vec!["ui\0x"],
        vec!["ui\u{1}x"],
        vec!["ui\0"],
        vec!["ui\u{1}"],
        vec!["ui\0x", "deeper"],
    ] {
        assert!(
            !key(&beside).under(&under),
            "{beside:?} read as a key under `ui`"
        );

        let (low, high) = under.subtree();
        let high = high.expect("a path that is not the root has a top");
        let held = key(&beside);
        let bytes = held.as_bytes();

        assert!(
            !(bytes >= low && bytes < high.as_slice()),
            "{beside:?} fell inside the range of `ui`"
        );
    }

    assert!(key(&["ui", "\0x"]).under(&under));
}

#[test]
fn a_level_holding_those_bytes_still_reads_back() {
    for path in [
        vec!["ui\0x"],
        vec!["ui\u{1}x"],
        vec!["\u{1}\u{1}", "\0\u{1}\0"],
        vec!["a", "\u{1}", "b"],
    ] {
        assert_eq!(levels(&key(&path)), path, "{path:?} did not read back");
    }
}

#[test]
fn everything_is_under_the_root() {
    let root = Key::root();

    for path in [vec!["a"], vec!["a", "b"], vec!["\0"]] {
        assert!(key(&path).under(&root), "{path:?}");
    }
}

#[test]
fn the_range_holds_the_subtree_and_stops_at_it() {
    let under = key(&["pot"]);
    let (low, high) = under.subtree();
    let high = high.expect("a path that is not the root has a top");

    let held = |path: &[&str]| {
        let key = key(path);
        let bytes = key.as_bytes().to_vec();
        bytes.as_slice() >= low && bytes.as_slice() < high.as_slice()
    };

    assert!(held(&["pot"]));
    assert!(held(&["pot", "ato"]));
    assert!(held(&["pot", "zzz", "deep"]));

    assert!(!held(&["potato"]));
    assert!(!held(&["pot!luck"]));
    assert!(!held(&["por"]));
    assert!(!held(&["pou"]));
}

#[test]
fn the_root_has_no_top() {
    let root = Key::root();
    let (low, high) = root.subtree();

    assert_eq!(low, b"");
    assert!(high.is_none());
}

#[test]
fn a_key_no_path_can_spell_is_refused() {
    let unterminated = key(&["ui"]);
    let mut bytes = unterminated.as_bytes().to_vec();
    bytes.pop();

    assert!(matches!(
        rebuilt(&bytes).unwrap_err(),
        StorePathError::DanglingEscape
    ));

    assert!(rebuilt(&[0xFF, 0xFE, 0x00]).is_err());
}

#[test]
fn a_level_with_no_name_reads_back_as_one() {
    let read = rebuilt(b"ui\0\0theme\0").unwrap();

    assert_eq!(read.len(), 3);
    assert_eq!(read.segment_at(1).unwrap().as_str(), "");
    assert_eq!(read, StorePath::from_segments(["ui", "", "theme"]));
}

fn rebuilt(bytes: &[u8]) -> Result<StorePath, StorePathError> {
    Key::from_bytes(bytes).path()
}
