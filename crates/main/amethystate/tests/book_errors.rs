use amethystate::errors::StorageError;
use amethystate::errors::facts::{self, Entry, Key, Prefix};
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{LoadMap, OpenStruct, StorageResult, WriteValue};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use error_stack::Report;
use std::error::Error;
use std::fmt;

mod common;

//@show getting at the report a set carries
fn what_the_store_said(why: LoadMap) -> StorageResult<()> {
    match why {
        LoadMap::EntryWillNotRead { why, .. } => Err(why.into_report()),
        other => panic!("an entry was expected to be at fault: {other}"),
    }
}
//@show-end

fn open(backend: Backend, tag: &str) -> anyhow::Result<(amethystate::Store, TempPath)> {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    Ok((store, path))
}

fn as_printed(report: &impl std::fmt::Debug) -> String {
    let dressed = format!("{report:?}");

    let mut plain = String::new();
    let mut chars = dressed.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for escape in chars.by_ref() {
                if escape == 'm' {
                    break;
                }
            }
            continue;
        }
        plain.push(c);
    }

    let mut kept: Vec<String> = plain
        .lines()
        .filter(|line| !line.contains("╴at "))
        .filter(|line| !line.contains("╴store: "))
        .filter(|line| !line.contains("╴meta file: "))
        .map(str::to_string)
        .collect();

    if let Some(last) = kept.last_mut()
        && let Some(rest) = last.strip_prefix("├╴")
    {
        *last = format!("╰╴{rest}");
    }

    kept.join("\n")
}

fn refusal(said: &impl fmt::Display, tree: Option<&Report<StorageError>>) -> String {
    let Some(report) = tree else {
        return said.to_string();
    };

    let printed = as_printed(report);
    let said = said.to_string();

    match printed.starts_with(&said) {
        true => printed,
        false => format!("{said}\n\n{printed}"),
    }
}

#[backends(all)]
fn a_constructor_fails_with_the_set_that_is_possible_there(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_set")?;

    //@show telling one refusal from another
    let refused = match Panel::new_with(&store) {
        Ok(_) => return Ok(()),
        Err(why) => why,
    };

    let said = match refused {
        OpenStruct::Refused { at, said } => format!("{at} was turned down: {said}"),
        OpenStruct::WillNotRead { at, why } => format!("{at} holds something else: {why}"),
        OpenStruct::Taken(taken) => format!("{} already holds it", taken.held_by),
        OpenStruct::NotAPath(why) => format!("that is not a path: {why}"),
        OpenStruct::Store(disk) => format!("the store: {disk}"),
    };
    //@show-end

    unreachable!("nothing here refuses the panel: {said}")
}

#[amethystate::amethystate(prefix = "panel")]
pub struct Panel {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[backends(all)]
fn the_top_of_a_report_names_the_operation(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_top")?;
    store.set(["labels", "cpu"], &"text".to_string())?;

    //@show what a failure says it is
    let refused = store.kv().map::<String, u64>("labels").unwrap_err();
    let report = what_the_store_said(refused).unwrap_err();

    let context = report.current_context();
    let sentence = report.to_string();
    //@show-end

    assert_eq!(context, &StorageError::Codec);
    assert_eq!(sentence, "the value could not be encoded or decoded");

    Ok(())
}

#[backends(all)]
fn a_report_carries_the_entry_it_failed_on(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_entry")?;
    store.set(["ports", "http"], &"text".to_string())?;

    //@show reaching the entry that failed
    let refused = store.kv().map::<String, u64>("ports").unwrap_err();
    let report = what_the_store_said(refused).unwrap_err();

    let entries: Vec<&Entry> = facts::all::<Entry, _>(&report).collect();
    let prefixes: Vec<&Prefix> = facts::all::<Prefix, _>(&report).collect();
    //@show-end

    assert_eq!(entries[0].0, "http");
    assert_eq!(prefixes[0].0.to_string(), "ports");

    Ok(())
}

#[backends(all)]
fn a_fact_that_is_not_there_reads_as_nothing(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_absent")?;
    store.set(["ports", "http"], &"text".to_string())?;

    //@show asking for a fact the report does not carry
    let refused = store.kv().map::<String, u64>("ports").unwrap_err();
    let report = what_the_store_said(refused).unwrap_err();

    let key = facts::all::<Key, _>(&report).next();
    //@show-end

    assert!(key.is_none());

    Ok(())
}

#[backends(all)]
fn the_whole_chain_is_in_the_debug_form(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_chain")?;
    store.set(["ports", "http"], &"text".to_string())?;

    let refused = store.kv().map::<String, u64>("ports").unwrap_err();
    let report = what_the_store_said(refused).unwrap_err();

    let sentence = report.to_string();
    let whole = format!("{report:?}");

    assert!(!sentence.contains("entry: http"));
    assert!(whole.contains("entry: http"));
    assert!(whole.contains(&StorageError::Codec.to_string()));

    Ok(())
}

#[backends(all)]
fn a_variant_that_classified_a_report_still_holds_it(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_classified")?;
    store.set(["port"], &"not a number".to_string())?;

    //@show the report under a variant that named the failure
    let refused = store.get::<u16>(["port"]).unwrap_err();

    let amethystate::store::ReadValue::WillNotRead { at, why } = refused else {
        panic!("the bytes are there and they are not a u16")
    };
    //@show-end

    assert_eq!(at.to_string(), "port");
    assert!(
        format!("{why:?}").contains("key: port"),
        "the report the variant classified is still whole: {why:?}"
    );

    Ok(())
}

#[backends(all)]
fn into_error_keeps_what_the_report_carried(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_into")?;
    store.set(["ports", "http"], &"text".to_string())?;

    let refused = store.kv().map::<String, u64>("ports").unwrap_err();
    let report = what_the_store_said(refused).unwrap_err();

    //@show turning a report into a std error
    let std_error = report.into_error();

    let sentence = std_error.to_string();
    let whole = format!("{std_error:?}");
    //@show-end

    assert_eq!(sentence, "the value could not be encoded or decoded");
    assert!(whole.contains("entry: http"));

    Ok(())
}

#[backends(Redb)]
fn what_different_refusals_look_like(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_shapes")?;

    //@show an entry that will not decode
    store.set(["ports", "http"], &"text".to_string())?;

    let undecodable = store.kv().map::<String, u64>("ports").unwrap_err();
    //@show-end

    //@show an entry whose name is not the map's key type
    let wrong_key = store.kv().map::<u16, String>("ports").unwrap_err();
    //@show-end

    //@show a name that cannot be a level
    let empty_level = store.set([""], &1u32).unwrap_err();
    //@show-end

    let capped = TempPath::new("book_err_depth");
    let settings = capped.path();

    //@show a path past the cap it was given
    let shallow = StoreBuilder::new(settings)
        .limits(|l| l.key_depth(4))
        .build()?;

    let too_deep = shallow.set(["a", "b", "c", "d", "e"], &1u32).unwrap_err();
    //@show-end

    let LoadMap::EntryWillNotRead { why: disk, .. } = &undecodable else {
        panic!("{undecodable}")
    };
    let WriteValue::TooDeep { why: budget, .. } = &too_deep else {
        panic!("{too_deep}")
    };

    common::measured(&[
        ("what failed", "a refusal"),
        (
            "an entry that will not decode",
            &refusal(&undecodable, Some(disk)),
        ),
        (
            "an entry whose name is not the map's key type",
            &refusal(&wrong_key, None),
        ),
        (
            "a name that cannot be a level",
            &refusal(&empty_level, None),
        ),
        (
            "a path past the cap it was given",
            &refusal(&too_deep, Some(budget)),
        ),
    ]);

    Ok(())
}

#[backends(all)]
fn a_refusal_travels_as_whatever_the_caller_already_uses(backend: Backend) -> anyhow::Result<()> {
    let (store, _path) = open(backend, "book_err_boxed")?;

    //@show letting the caller's own error type take it
    fn with_anyhow(store: &amethystate::Store) -> anyhow::Result<()> {
        store.set(["ui", "width"], &800u32)?;
        Ok(())
    }

    fn with_a_box(store: &amethystate::Store) -> Result<(), Box<dyn Error + Send + Sync>> {
        store.set(["ui", "height"], &600u32)?;
        Ok(())
    }
    //@show-end

    with_anyhow(&store)?;
    with_a_box(&store).map_err(|why| anyhow::anyhow!("{why}"))?;

    assert_eq!(store.get::<u32>(["ui", "width"])?, Some(800));

    Ok(())
}
