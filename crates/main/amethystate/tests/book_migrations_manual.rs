use amethystate::StoreBuilder;
use amethystate::migration::{ComponentOutcome, MigrationError};
use amethystate::store::OpenStore;
use amethystate::store::builder::Backend;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(Redb)]
fn steps_registered_by_hand_run_through_build_with_migration(
    _backend: Backend,
) -> anyhow::Result<()> {
    let path = TempPath::new("book_manual_register");
    let app = path.path();

    {
        let store = StoreBuilder::new(app).build()?;
        store.set(["net", "port"], &80u16)?;
        store.close()?;
    }

    //@show registering steps by hand
    let (store, report) = StoreBuilder::new(app)
        .migrations(|m| {
            m.for_prefix("net")
                .step(1, "move off the privileged port", |ctx| {
                    ctx.set("port", &8080u16)
                });
        })
        .build_with_migration()?;
    //@show-end

    assert!(!report.has_failures(), "{report:?}");
    assert_eq!(store.get::<u16>(["net", "port"])?, Some(8080));
    Ok(())
}

#[backends(Redb)]
fn a_step_reads_what_the_application_provided(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_manual_provide");
    let app = path.path();

    {
        let store = StoreBuilder::new(app).build()?;
        store.set(["net", "host"], &"localhost".to_string())?;
        store.close()?;
    }

    //@show a value the application provides
    struct LegacyDefaults {
        port: u16,
    }

    let (store, report) = StoreBuilder::new(app)
        .provide(LegacyDefaults { port: 8080 })
        .migrations(|m| {
            m.for_prefix("net")
                .step(1, "fill in the port the old build assumed", |ctx| {
                    let port = ctx.require::<LegacyDefaults>()?.port;
                    ctx.set("port", &port)
                });
        })
        .build_with_migration()?;
    //@show-end

    assert!(!report.has_failures(), "{report:?}");
    assert_eq!(store.get::<u16>(["net", "port"])?, Some(8080));
    Ok(())
}

#[backends(Redb)]
fn a_failed_step_refuses_build_and_is_reported_by_build_with_migration(
    _backend: Backend,
) -> anyhow::Result<()> {
    let path = TempPath::new("book_manual_failed");
    let app = path.path();

    {
        let store = StoreBuilder::new(app).build()?;
        store.set(["net", "port"], &80u16)?;
        store.close()?;
    }

    //@show a step that fails refuses the open
    let opened = StoreBuilder::new(app)
        .migrations(|m| {
            m.for_prefix("net").step(1, "turns the data down", |_| {
                Err(MigrationError::Custom("this data is not ours".into()).into())
            });
        })
        .build();

    assert!(matches!(opened, Err(OpenStore::Migrating { .. })));
    //@show-end

    drop(opened);

    //@show opening anyway, and reading what failed
    let (store, report) = StoreBuilder::new(app)
        .migrations(|m| {
            m.for_prefix("net").step(1, "turns the data down", |_| {
                Err(MigrationError::Custom("this data is not ours".into()).into())
            });
        })
        .build_with_migration()?;

    for component in &report.components {
        if let ComponentOutcome::Failed { error, .. } = &component.outcome {
            eprintln!("{:?} was left as it was: {error:?}", component.prefixes);
        }
    }
    //@show-end

    assert!(report.has_failures());
    assert_eq!(store.get::<u16>(["net", "port"])?, Some(80));
    Ok(())
}
