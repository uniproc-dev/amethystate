use amethystate::store::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder, default_backend};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;

#[backends(Redb)]
fn a_path_deeper_than_the_cap_is_refused(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_limits_depth");
    let settings = path.path();

    //@show capping how deep a path may go
    let store = StoreBuilder::new(settings)
        .limits(|l| l.key_depth(4))
        .build()?;

    let deep = StorePath::from_segments(["a", "b", "c", "d", "e"]);

    if let Err(refused) = store.set(&deep, &1u32) {
        println!("{refused:?}");
    }
    //@show-end

    let refused = store.set(&deep, &1u32).unwrap_err();
    assert_eq!(refused.current_context(), &StorageError::Depth);

    store.set(StorePath::from_segments(["a", "b", "c", "d"]), &1u32)?;

    Ok(())
}

#[backends(Redb)]
fn a_store_can_promise_to_stay_readable_elsewhere(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_limits_portable");
    let settings = path.path();

    //@show promising the contents stay readable on another engine
    let store = StoreBuilder::new(settings)
        .limits(|l| l.portable_across([default_backend()]))
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;

    Ok(())
}

#[test]
fn what_each_engine_reads_at_most() {
    //@show how deep the engine you are running reads
    let engine = default_backend();
    println!("{}: {} levels", engine.extension(), engine.depth_ceiling());
    //@show-end
}
