use amethystate::store::WriteValue;
use amethystate::store::builder::{Backend, StoreBuilder, default_backend};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(Redb)]
fn a_path_deeper_than_the_cap_is_refused(_backend: Backend) -> anyhow::Result<()> {
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
    assert!(matches!(refused, WriteValue::TooDeep { .. }), "{refused}");

    store.set(StorePath::from_segments(["a", "b", "c", "d"]), &1u32)?;

    Ok(())
}

#[backends(Redb)]
fn a_store_can_promise_to_stay_readable_elsewhere(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_limits_portable");
    let settings = path.path();

    //@show promising the contents stay readable on another engine
    let store = StoreBuilder::new(settings)
        .limits(|l| l.portable_across([default_backend()]))
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;

    #[cfg(all(feature = "redb", feature = "json"))]
    {
        let path = TempPath::new("book_limits_portable_json");
        let strict = StoreBuilder::new(path.path())
            .backend(Backend::Redb)
            .limits(|l| l.portable_across([Backend::Json]))
            .build()?;

        let refused = strict
            .kv()
            .set("ratio", &f64::NAN)
            .expect_err("a store promising JSON must refuse what JSON cannot hold");
        assert!(
            format!("{refused:?}").contains("json"),
            "the refusal must name the engine that could not hold it: {refused:?}"
        );
    }

    Ok(())
}

#[test]
fn what_each_engine_reads_at_most() -> anyhow::Result<()> {
    //@show how deep the engine you are running reads
    let engine = default_backend();
    println!("{}: {} levels", engine.extension(), engine.depth_ceiling());
    //@show-end

    let ceiling = engine.depth_ceiling();
    let path = TempPath::new("book_limits_ceiling");
    let store = StoreBuilder::new(path.path()).backend(engine).build()?;

    let at = StorePath::from_segments(["deep"]);
    store.set(&at, &1u32)?;

    let mut nested = serde_json::Value::from(1u32);
    for _ in 0..ceiling {
        nested = serde_json::Value::Array(vec![nested]);
    }

    let refused = store
        .set(&at, &nested)
        .expect_err("a value nested past the ceiling must be refused");
    assert!(
        format!("{refused:?}").contains(&ceiling.to_string()),
        "the refusal must name the ceiling it was measured against: {refused:?}"
    );

    Ok(())
}
