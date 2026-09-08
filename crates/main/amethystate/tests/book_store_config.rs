use amethystate::store::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder, default_backend};
use amethystate::store::config::AfterGivingUp;
use amethystate::store::config::WriteAttempts;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::time::Duration;

#[backends(Redb)]
fn a_store_given_both_intervals_opens_and_writes(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_config_intervals");
    let settings = path.path();

    //@show how long a write waits, and how long an outside edit settles
    let store = StoreBuilder::new(settings)
        .disk(|d| {
            d.debounce(Duration::from_millis(500))
                .watch_every(Duration::from_secs(2))
        })
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));
    Ok(())
}

#[backends(Redb)]
fn a_store_given_a_retry_policy_opens_and_writes(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_config_retry");
    let settings = path.path();

    //@show how long a failing flush stays quiet
    let store = StoreBuilder::new(settings)
        .disk(|d| {
            d.retry_every(Duration::from_millis(200))
                .give_up_after(Duration::from_secs(10))
                .on_failure(|failure| match failure.current_context() {
                    StorageError::Codec => AfterGivingUp::Poison,
                    _ => AfterGivingUp::Ignore,
                })
        })
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));
    Ok(())
}

#[backends(Redb)]
fn a_store_given_a_write_policy_opens_and_writes(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_config_write");
    let settings = path.path();

    //@show how hard one write to one file fights
    let store = StoreBuilder::new(settings)
        .file_write(|w| {
            w.writing(WriteAttempts::times(3).apart(Duration::from_millis(50)))
                .replacing(WriteAttempts::times(20).apart(Duration::from_millis(250)))
        })
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));
    Ok(())
}

#[backends(Redb)]
fn a_store_can_refuse_what_another_engine_could_not_hold(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_config_limits");
    let settings = path.path();

    //@show refusing what this store should not hold
    let store = StoreBuilder::new(settings)
        .limits(|l| l.key_depth(8).portable_across([default_backend()]))
        .build()?;
    //@show-end

    store.set(["a", "b", "c", "d", "e", "f", "g", "h"], &1u8)?;

    let too_deep = store
        .set(["a", "b", "c", "d", "e", "f", "g", "h", "i"], &1u8)
        .expect_err("a ninth level is past the eight this store was given");
    assert!(
        format!("{too_deep}").contains("depth") || format!("{too_deep:?}").contains("depth"),
        "the refusal must name the depth that stopped it: {too_deep:?}"
    );

    Ok(())
}

#[backends(Redb)]
fn reading_a_large_collection_can_use_more_than_one_core(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_config_parallel");
    let settings = path.path();

    //@show letting a large read use more than one core
    let store = StoreBuilder::new(settings).parallel_reads(true).build()?;
    //@show-end

    assert!(
        store.parallel_reads(),
        "the store was asked for parallel reads and does not report them"
    );

    store.kv().set("port", &8080u16)?;
    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));
    Ok(())
}
