use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "relabel", version = 1)]
    pub struct Relabel {
        pub before: u32,
    }
}

#[amethystate(prefix = "relabel", version = 2)]
pub struct Relabel {
    pub after: u32,
}

#[migrate]
#[rename(before => after)]
fn migrate_relabel_v1_to_v2(
    old: AmeData<v1::Relabel>,
) -> amethystate::MigrationResult<AmeData<Relabel>> {
    Ok(AmeData::<Relabel> { after: old.before })
}

#[backends(all)]
fn every_place_moved_by_a_step_leaves_no_old_declaration_standing(backend: Backend) {
    let at = TempPath::new("relabel");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        v1::Relabel::new_with(&store)
            .unwrap()
            .before()
            .set(42)
            .unwrap();
        store.save_now().unwrap();
    }

    for open in 0..2 {
        let (store, report) = StoreBuilder::new(at.path())
            .backend(backend)
            .migrations(|m| {
                m.collect_codegen();
            })
            .migrate()
            .unwrap();

        assert!(
            !report.has_failures(),
            "{backend:?} open {open}: {report:?}"
        );
        assert!(!report.has_drift(), "{backend:?} open {open}: {report:?}");
        assert_eq!(
            Relabel::new_with(&store).unwrap().after().get(),
            42,
            "{backend:?} open {open}"
        );
    }
}
