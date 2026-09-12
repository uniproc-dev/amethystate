use amethystate::migration::error::MigrationError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(all)]
fn a_reach_reads_the_value_the_other_prefix_was_migrated_to(backend: Backend) {
    let path = TempPath::new("reach_orders");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["net", "port"], &80u16).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(path.path())
        .backend(backend)
        .migrations(|m| {
            m.for_prefix("app")
                .step(1, "adopt the settled port", |ctx| {
                    let port = ctx.global_get::<u16>("net.port")?.unwrap_or(0);
                    ctx.set("proxy_port", &port)
                });

            m.for_prefix("net")
                .step(1, "move off the privileged port", |ctx| {
                    ctx.set("port", &8080u16)
                });
        })
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures());
    assert_eq!(
        store.get::<u16>(["app", "proxy_port"]).unwrap(),
        Some(8080),
        "`net` sorts after `app`, so only the reach could have put it first"
    );
}

#[backends(all)]
fn two_prefixes_reaching_into_each_other_are_named_end_to_end(backend: Backend) {
    let path = TempPath::new("reach_cycle");

    let (_store, report) = StoreBuilder::new(path.path())
        .backend(backend)
        .migrations(|m| {
            m.for_prefix("alpha").step(1, "reach into beta", |ctx| {
                ctx.global_get::<u32>("beta.thing")?;
                Ok(())
            });
            m.for_prefix("beta").step(1, "reach back", |ctx| {
                ctx.global_get::<u32>("alpha.thing")?;
                Ok(())
            });
        })
        .build_with_migration()
        .unwrap();

    let failure = report
        .components
        .iter()
        .find_map(|pass| match &pass.outcome {
            amethystate::migration::ComponentOutcome::Failed { error } => Some(error),
            _ => None,
        })
        .expect("a cycle has to be reported as a failure");

    let cycle = failure
        .downcast_ref::<MigrationError>()
        .expect("and as a migration failure");

    assert_eq!(
        format!("{cycle}"),
        "a migration reached round to where it started: alpha -> beta -> alpha"
    );
}
