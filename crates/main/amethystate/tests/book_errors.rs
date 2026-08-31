use amethystate::errors::facts::{self, Entry, Key, Prefix};
use amethystate::errors::{StorageError, WriteError};
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;

mod common;

fn open(
    backend: Backend,
    tag: &str,
) -> Result<(amethystate::Store, TempPath), Box<dyn Error + Send + Sync>> {
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

#[backends(all)]
fn the_top_of_a_report_names_the_operation(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_top")?;
    store.set(["labels", "cpu"], &"text".to_string())?;

    //@show what a failure says it is
    let refused = store.kv().map::<String, u64>("labels").unwrap_err();

    let context = refused.current_context();
    let sentence = refused.to_string();
    //@show-end

    assert_eq!(context, &WriteError::Storage);
    assert_eq!(sentence, "the store could not carry out the write");

    Ok(())
}

#[backends(all)]
fn a_report_carries_the_entry_it_failed_on(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_entry")?;
    store.set(["ports", "http"], &1u64)?;

    //@show reaching the entry that failed
    let refused = store.kv().map::<u16, u64>("ports").unwrap_err();

    let entries: Vec<&Entry> = facts::all::<Entry, _>(&refused).collect();
    let prefixes: Vec<&Prefix> = facts::all::<Prefix, _>(&refused).collect();
    //@show-end

    assert_eq!(entries[0].0, "http");
    assert_eq!(prefixes[0].0.to_string(), "ports");

    Ok(())
}

#[backends(all)]
fn a_fact_that_is_not_there_reads_as_nothing(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_absent")?;
    store.set(["ports", "http"], &1u64)?;

    //@show asking for a fact the report does not carry
    let refused = store.kv().map::<u16, u64>("ports").unwrap_err();

    let key = facts::all::<Key, _>(&refused).next();
    //@show-end

    assert!(key.is_none());

    Ok(())
}

#[backends(all)]
fn the_whole_chain_is_in_the_debug_form(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_chain")?;
    store.set(["ports", "http"], &1u64)?;

    let refused = store.kv().map::<u16, u64>("ports").unwrap_err();

    let sentence = refused.to_string();
    let whole = format!("{refused:?}");

    assert!(!sentence.contains("entry: http"));
    assert!(whole.contains("entry: http"));
    assert!(whole.contains(&StorageError::Codec.to_string()));

    Ok(())
}

#[backends(all)]
fn into_error_keeps_what_the_report_carried(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_into")?;
    store.set(["ports", "http"], &1u64)?;

    let refused = store.kv().map::<u16, u64>("ports").unwrap_err();

    //@show turning a report into a std error
    let std_error = refused.into_error();

    let sentence = std_error.to_string();
    let whole = format!("{std_error:?}");
    //@show-end

    assert_eq!(sentence, "the store could not carry out the write");
    assert!(whole.contains("entry: http"));

    Ok(())
}

#[backends(Redb)]
fn what_different_refusals_look_like(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_shapes")?;

    //@show an entry that will not decode
    store.set(["ports", "http"], &1u64)?;

    let undecodable = store.kv().map::<u16, u64>("ports").unwrap_err();
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

    common::measured(&[
        ("what failed", "a refusal"),
        ("an entry that will not decode", &as_printed(&undecodable)),
        ("a name that cannot be a level", &as_printed(&empty_level)),
        ("a path past the cap it was given", &as_printed(&too_deep)),
    ]);

    Ok(())
}

#[backends(all)]
fn a_report_travels_as_a_boxed_error(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_err_boxed")?;

    //@show handing a report to something that wants a std error
    fn writing(store: &amethystate::Store) -> Result<(), Box<dyn Error + Send + Sync>> {
        store.set(["ui", "width"], &800u32)?;
        Ok(())
    }
    //@show-end

    writing(&store)?;

    assert_eq!(store.get::<u32>(["ui", "width"])?, Some(800));

    Ok(())
}
