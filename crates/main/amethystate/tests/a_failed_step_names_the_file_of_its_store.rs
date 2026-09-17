use amethystate::amethystate;
use amethystate::migration::{ComponentOutcome, MigrationError};
use amethystate::store::OpenStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "located", version = 2)]
pub struct Panel {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[backends(all)]
fn a_step_that_fails_is_reported_with_the_file_of_the_store_it_was_in(backend: Backend) {
    let at = TempPath::new("failed_step_file");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["located", "width"], &640u32).unwrap();
        store.close().unwrap();
    }

    let Err(OpenStore::Migrating {
        report: Some(report),
        ..
    }) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.for_node::<Panel>().step(2, "turns the data down", |_| {
                Err(MigrationError::Custom("this data is not ours".into()).into())
            });
        })
        .migrate()
    else {
        panic!("{backend:?}: a failed step refuses the open with its report");
    };

    let failed = report
        .components
        .iter()
        .find_map(|one| match &one.outcome {
            ComponentOutcome::Failed { error, .. } => Some(format!("{error:?}")),
            _ => None,
        })
        .expect("the step returned an error");

    let file = at
        .path()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(failed.contains(&file), "{backend:?}: {failed}");
}
