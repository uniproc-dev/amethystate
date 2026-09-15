use amethystate::StoreBackend;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::format::{StorageFactSet, TestFormatRecord};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod common;

fn record(store: &amethystate::Store) -> &dyn TestFormatRecord {
    StoreBackend::format_record(store).expect("every engine in this crate keeps one")
}

trait Said<T> {
    fn said(self) -> anyhow::Result<T>;
}

impl<T> Said<T> for amethystate::StorageResult<T> {
    fn said(self) -> anyhow::Result<T> {
        self.map_err(|why| anyhow::Error::new(why.into_error()))
    }
}

#[backends(all)]
fn a_new_store_records_how_it_was_written(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("facts_new");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    let recorded = record(&store)
        .facts()
        .said()?
        .expect("a store this build opened records what it wrote");

    assert_eq!(recorded, StorageFactSet::of(backend));

    store.close()?;
    Ok(())
}

#[backends(all)]
fn a_deciding_fact_this_build_does_not_know_refuses_the_open(
    backend: Backend,
) -> anyhow::Result<()> {
    let path = TempPath::new("facts_unknown");

    {
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        record(&store)
            .set_facts(&StorageFactSet::of(backend).with("codec.frames", "chunked"))
            .said()?;
        store.close()?;
    }

    let refused = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .expect_err("the store records a deciding fact this build has no name for");

    let printed = format!("{refused:?}");
    assert!(
        printed.contains("codec.frames"),
        "the refusal names the fact that stopped it: {printed}"
    );

    Ok(())
}

#[backends(all)]
fn a_known_fact_at_a_value_this_build_does_not_write_refuses_too(
    backend: Backend,
) -> anyhow::Result<()> {
    let path = TempPath::new("facts_value");

    {
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        record(&store)
            .set_facts(&StorageFactSet::of(backend).with("path.sep", "/"))
            .said()?;
        store.close()?;
    }

    let refused = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .expect_err("`/` is not the separator this build writes");

    let printed = format!("{refused:?}");
    assert!(
        printed.contains("path.sep"),
        "the refusal names the fact rather than the value alone: {printed}"
    );

    Ok(())
}

#[backends(all)]
fn a_fact_outside_the_deciding_namespaces_opens_and_survives(
    backend: Backend,
) -> anyhow::Result<()> {
    let path = TempPath::new("facts_kept");

    {
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        record(&store)
            .set_facts(&StorageFactSet::of(backend).with("wrote.by", "a build from 2027"))
            .said()?;
        store.close()?;
    }

    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    store.set(["kept", "something"], &1u32)?;
    store.save_now()?;

    let recorded = record(&store)
        .facts()
        .said()?
        .expect("the record is still there");
    assert_eq!(
        recorded.get("wrote.by"),
        Some("a build from 2027"),
        "a build never removes what it does not understand"
    );

    store.close()?;
    Ok(())
}

#[backends(all)]
fn an_empty_record_is_filled_in_on_the_way_through(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("facts_empty");

    {
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        store.set(["old", "value"], &7u32)?;
        record(&store)
            .set_facts(&StorageFactSet::default())
            .said()?;
        store.close()?;
    }

    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    assert_eq!(store.get::<u32>(["old", "value"])?, Some(7));
    assert_eq!(
        record(&store).facts().said()?,
        Some(StorageFactSet::of(backend)),
        "an empty set decides nothing, so the open fills it in rather than refusing"
    );

    store.close()?;
    Ok(())
}
