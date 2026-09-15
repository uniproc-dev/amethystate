use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, migrate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod moved_v1 {
    use super::*;

    #[amethystate(prefix = "moved", version = 1)]
    pub struct Moved {
        #[amestate(path = "slot", default = 1u32)]
        pub alpha: u32,
    }
}

#[amethystate(prefix = "moved", version = 2)]
pub struct Moved {
    #[amestate(default = 1u32)]
    pub beta: u32,

    pub slot: ReactiveMap<String, u32>,
}

#[migrate]
#[rename(alpha => beta)]
fn migrate_moved_v1_to_v2(
    old: AmeData<moved_v1::Moved>,
) -> amethystate::MigrationResult<AmeData<Moved>> {
    Ok(AmeData::<Moved> {
        beta: old.alpha,
        slot: Default::default(),
    })
}

#[backends(all)]
fn a_renamed_field_takes_its_stored_place_with_it(backend: Backend) {
    let path = TempPath::new("moved");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let v1 = moved_v1::Moved::new_with(&store).unwrap();
        v1.alpha().set(42).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
        assert_eq!(store.get::<u32>(["moved", "slot"]).unwrap(), Some(42));
    }

    let (store, _report) = StoreBuilder::new(path.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert_eq!(
        store.get::<u32>(["moved", "slot"]).unwrap(),
        None,
        "the place the renamed field was stored at outlived the rename"
    );

    let v2 = Moved::new_with(&store).unwrap();
    assert_eq!(v2.beta().get(), 42);
}
