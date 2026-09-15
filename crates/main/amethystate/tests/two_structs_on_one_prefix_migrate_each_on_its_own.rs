use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "pair", id = "left", version = 1)]
    pub struct Left {
        pub left: u32,
    }

    #[amethystate(prefix = "pair", id = "right", version = 1)]
    pub struct Right {
        pub right: u32,
    }
}

#[amethystate(prefix = "pair", id = "left", version = 2)]
pub struct Left {
    pub left: u32,
    pub doubled: u32,
}

#[amethystate(prefix = "pair", id = "right", version = 2)]
pub struct Right {
    pub right: String,
}

#[migrate]
fn migrate_left_v1_to_v2(old: AmeData<v1::Left>) -> amethystate::MigrationResult<AmeData<Left>> {
    Ok(AmeData::<Left> {
        left: old.left,
        doubled: old.left * 2,
    })
}

#[migrate]
fn migrate_right_v1_to_v2(old: AmeData<v1::Right>) -> amethystate::MigrationResult<AmeData<Right>> {
    Ok(AmeData::<Right> {
        right: old.right.to_string(),
    })
}

#[backends(all)]
fn two_structs_on_one_prefix_migrate_each_on_its_own(backend: Backend) {
    let at = TempPath::new("pair");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        v1::Left::new_with(&store).unwrap().left().set(7).unwrap();
        v1::Right::new_with(&store).unwrap().right().set(9).unwrap();
        store.save_now().unwrap();
    }

    for open in 0..2 {
        let (store, report) = StoreBuilder::new(at.path())
            .backend(backend)
            .migrations(|m| {
                m.collect_codegen();
            })
            .build_with_migration()
            .unwrap();

        assert!(
            !report.has_failures(),
            "{backend:?} open {open}: {report:?}"
        );
        assert!(!report.has_drift(), "{backend:?} open {open}: {report:?}");

        let left = Left::new_with(&store).unwrap();
        assert_eq!(left.left().get(), 7, "{backend:?} open {open}");
        assert_eq!(left.doubled().get(), 14, "{backend:?} open {open}");
        assert_eq!(
            Right::new_with(&store).unwrap().right().get(),
            "9",
            "{backend:?} open {open}"
        );
    }
}
