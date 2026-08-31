use amethystate::store::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder, default_backend};
use amethystate::store::config::AfterGivingUp;
use amethystate::store::config::WriteAttempts;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;
use std::time::Duration;

#[backends(Redb)]
fn the_two_intervals_are_set_apart(_backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    Ok(())
}

#[backends(Redb)]
fn a_failing_flush_is_retried_and_then_reported(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    Ok(())
}

#[backends(Redb)]
fn one_write_can_be_told_how_hard_to_fight(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
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
    Ok(())
}

#[backends(Redb)]
fn a_store_can_refuse_what_another_engine_could_not_hold(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_config_limits");
    let settings = path.path();

    //@show refusing what this store should not hold
    let store = StoreBuilder::new(settings)
        .limits(|l| l.key_depth(8).portable_across([default_backend()]))
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    Ok(())
}

#[backends(Redb)]
fn reading_a_large_collection_can_use_more_than_one_core(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_config_parallel");
    let settings = path.path();

    //@show letting a large read use more than one core
    let store = StoreBuilder::new(settings).parallel_reads(true).build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    Ok(())
}
