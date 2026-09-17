use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "twice", version = 1)]
    pub struct Twice {
        pub count: u32,
    }
}

#[amethystate(prefix = "twice", version = 2)]
pub struct Twice {
    pub count: u32,
    pub doubled: u32,
}

#[migrate]
fn doubles_it(old: AmeData<v1::Twice>) -> amethystate::MigrationResult<AmeData<Twice>> {
    Ok(AmeData::<Twice> {
        count: old.count,
        doubled: old.count * 2,
    })
}

#[backends(all)]
fn two_steps_to_one_version_are_not_one_quietly(backend: Backend) {
    let at = TempPath::new("twice");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        v1::Twice::new_with(&store).unwrap().count().set(7).unwrap();
        store.save_now().unwrap();
    }

    let opened = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
            m.for_node::<Twice>()
                .step(2, "zeroes it", |ctx| ctx.set("doubled", &0u32));
        })
        .migrate();

    let Ok((_store, report)) = opened else {
        return;
    };

    assert!(
        report.has_failures(),
        "{backend:?}: two steps take `twice` to v2 and one of them ran while the other \
         was passed over without a word: {report:?}"
    );
}
