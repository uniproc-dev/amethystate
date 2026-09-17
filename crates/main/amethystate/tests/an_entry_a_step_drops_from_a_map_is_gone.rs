use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, amethystate, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "trimmed", version = 1)]
    pub struct Trimmed {
        #[amestate(default = { "base": 0 })]
        pub limits: ReactiveMap<String, u32>,
    }
}

#[amethystate(prefix = "trimmed", version = 2)]
pub struct Trimmed {
    #[amestate(default = { "base": 0 })]
    pub limits: ReactiveMap<String, u32>,
}

#[migrate]
fn trim_v1_to_v2(old: AmeData<v1::Trimmed>) -> amethystate::MigrationResult<AmeData<Trimmed>> {
    let mut limits = old.limits;
    limits.shift_remove("gone");
    Ok(AmeData::<Trimmed> { limits })
}

#[backends(all)]
fn an_entry_a_step_drops_from_a_map_is_gone_after_it_runs(backend: Backend) {
    let at = TempPath::new("step_drops_an_entry");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let old = v1::Trimmed::new_with(&store).unwrap();
        old.limits().insert("gone".to_string(), &1).unwrap();
        old.limits().insert("kept".to_string(), &2).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrate()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    assert_eq!(
        store.get::<u32>(["trimmed", "limits", "gone"]).unwrap(),
        None,
        "{backend:?}"
    );
    assert_eq!(
        store.get::<u32>(["trimmed", "limits", "kept"]).unwrap(),
        Some(2),
        "{backend:?}"
    );
}
