#![cfg(feature = "redb")]

use amethystate::store::InspectorBackend;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;

mod v1 {
    use super::*;

    #[amethystate(prefix = "labels", version = 1)]
    pub struct Alpha {
        #[amestate(default = 1u32)]
        pub alpha_one: u32,
    }
}

#[amethystate(prefix = "labels", version = 2)]
pub struct Alpha {
    #[amestate(default = 1u32)]
    pub alpha_one: u32,
}

#[amethystate(prefix = "labels", id = "beta", version = 1)]
pub struct Beta {
    #[amestate(default = 2u32)]
    pub beta_one: u32,
}

#[migrate]
fn alpha_v1_to_v2(old: AmeData<v1::Alpha>) -> amethystate::MigrationResult<AmeData<Alpha>> {
    Ok(AmeData::<Alpha> {
        alpha_one: old.alpha_one,
    })
}

#[test]
fn a_record_names_the_struct_whose_fields_it_holds() {
    let path = TempPath::new("snapshot_names");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Redb)
            .build()
            .unwrap();
        let _early = v1::Alpha::new_with(&store).unwrap();
        store.save_now().unwrap();
    }

    {
        let (store, report) = StoreBuilder::new(path.path())
            .backend(Backend::Redb)
            .build_with_migration()
            .unwrap();
        assert!(!report.has_failures());
        let _alpha = Alpha::new_with(&store).unwrap();
        let _beta = Beta::new_with(&store).unwrap();
        store.save_now().unwrap();
    }

    let inspector = amethystate::stores::RedbStore::open(
        amethystate::StoreConfig::new(path.path()),
        Default::default(),
    )
    .unwrap()
    .0;

    let mut named: Vec<(String, Vec<String>)> = inspector
        .get_schema_snapshots()
        .unwrap()
        .into_iter()
        .filter(|(at, _)| at == "labels")
        .map(|(_, snapshot)| {
            (
                snapshot.struct_name.unwrap_or_default(),
                snapshot
                    .fields
                    .iter()
                    .map(|field| field.name.to_string())
                    .collect(),
            )
        })
        .collect();

    named.sort();

    assert_eq!(
        named,
        vec![
            ("Alpha".to_string(), vec!["alpha_one".to_string()]),
            ("Beta".to_string(), vec!["beta_one".to_string()]),
        ]
    );
}
