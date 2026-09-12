#![cfg(all(feature = "redb", feature = "toml"))]

use amethystate::StoreBuilder;
use amethystate::store::builder::Backend;
use amethystate_core::test_utils::TempPath;

fn raised<T>(what: Result<T, amethystate::errors::WriteValue>) -> String {
    let Err(why) = what else {
        panic!("the write was expected to fail");
    };

    report_only(&format!("{:?}", anyhow::Error::from(why)))
}

fn report_only(rendered: &str) -> String {
    let cut = ["\nStack backtrace:", "\nBacktrace No."]
        .iter()
        .filter_map(|marker| rendered.find(marker))
        .min()
        .unwrap_or(rendered.len());

    rendered[..cut].trim_end().to_string()
}

fn no_line_twice(rendered: &str) {
    let mut seen: Vec<&str> = Vec::new();

    for line in rendered.lines() {
        let line = line.trim().trim_start_matches(|c: char| c.is_ascii_digit());
        let line = line.trim_start_matches([':', ' ']);
        if line.is_empty() || line == "Caused by:" {
            continue;
        }
        assert!(
            !seen.contains(&line),
            "this line is in the output twice:\n{line}\n\nwhole:\n{rendered}"
        );
        seen.push(line);
    }
}

#[test]
fn a_refused_write_names_the_file_the_key_and_what_refused_it() {
    let at = TempPath::new("anyhow_deep");
    let store = StoreBuilder::new(at.path())
        .backend(Backend::Redb)
        .limits(|l| l.key_depth(3))
        .build()
        .unwrap();

    store.set(["a", "b", "c"], &1u32).unwrap();

    let rendered = raised(store.set(["a", "b", "c", "d"], &1u32));

    assert!(
        rendered.contains("anyhow_deep"),
        "the store's own file is not in it:\n{rendered}"
    );
    assert!(
        rendered.contains("a.b.c.d"),
        "the key is not in it:\n{rendered}"
    );
    assert!(
        rendered.contains("the limit is 3"),
        "what refused it is not in it:\n{rendered}"
    );
    no_line_twice(&rendered);
}

#[test]
fn a_codec_that_refuses_says_so_in_its_own_words() {
    let at = TempPath::new("anyhow_codec");
    let store = StoreBuilder::new(at.path())
        .backend(Backend::Toml)
        .build()
        .unwrap();

    let rendered = raised(store.set(["probe", "v"], &Some(None::<u32>)));

    assert!(
        rendered.contains("probe.v"),
        "the key is not in it:\n{rendered}"
    );
    assert!(
        rendered.to_lowercase().contains("toml"),
        "the engine's own words are not in it:\n{rendered}"
    );
    no_line_twice(&rendered);
}

#[test]
fn a_closed_store_says_it_once_and_carries_nothing_under_it() {
    let at = TempPath::new("anyhow_closed");
    let store = StoreBuilder::new(at.path())
        .backend(Backend::Redb)
        .build()
        .unwrap();
    store.close().unwrap();

    let rendered = raised(store.set(["a"], &1u32));

    assert_eq!(
        rendered.lines().count(),
        1,
        "a close is minted where it is refused, so there is nothing under it:\n{rendered}"
    );
    no_line_twice(&rendered);
}

#[test]
fn the_whole_report_is_there_for_a_caller_who_wants_all_of_it() {
    use amethystate::errors::WriteValue;

    let at = TempPath::new("anyhow_explain");
    let store = StoreBuilder::new(at.path())
        .backend(Backend::Redb)
        .limits(|l| l.key_depth(3))
        .build()
        .unwrap();

    let refused: WriteValue = store.set(["a", "b", "c", "d"], &1u32).unwrap_err();
    let explained = refused.explain();

    assert!(
        explained.contains("anyhow_explain") && explained.contains("a.b.c.d"),
        "explain() is meant to carry the facts:\n{explained}"
    );
}

#[test]
fn a_fact_reads_back_as_a_type_rather_than_as_a_sentence() {
    use amethystate::errors::{WriteValue, facts};

    let at = TempPath::new("anyhow_facts");
    let store = StoreBuilder::new(at.path())
        .backend(Backend::Redb)
        .limits(|l| l.key_depth(3))
        .build()
        .unwrap();

    let refused = store.set(["a", "b", "c", "d"], &1u32).unwrap_err();

    let WriteValue::TooDeep { why, .. } = &refused else {
        panic!("a path past the cap is refused for its depth: {refused}")
    };

    let held: Vec<&facts::Key> = facts::all(why).collect();
    assert!(
        held.iter().any(|key| key.0.to_string() == "a.b.c.d"),
        "the key is a fact and should come back as one: {refused:?}"
    );
}
