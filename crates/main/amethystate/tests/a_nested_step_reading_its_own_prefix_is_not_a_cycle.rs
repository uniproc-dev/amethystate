use amethystate::migration::MigrationContext;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate, migrate_field};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate]
    pub struct Net {
        pub port: u32,
    }

    #[amethystate(prefix = "sys", version = 1)]
    pub struct System {
        pub label: u32,

        #[amestate(nested)]
        pub net: Net,
    }
}

#[amethystate]
pub struct Net {
    pub port: u32,
    pub label_copy: u32,
}

#[amethystate(prefix = "sys", version = 2)]
pub struct System {
    pub label: u32,

    #[amestate(nested)]
    pub net: Net,
}

#[migrate]
fn migrate_net_v1_to_v2(
    old: AmeData<v1::Net>,
    ctx: &mut MigrationContext,
) -> amethystate::MigrationResult<AmeData<Net>> {
    Ok(AmeData::<Net> {
        port: old.port,
        label_copy: ctx.global_get::<u32>("sys.label")?.unwrap_or(0),
    })
}

#[migrate]
fn migrate_system_v1_to_v2(
    old: AmeData<v1::System>,
    ctx: &mut MigrationContext,
) -> amethystate::MigrationResult<AmeData<System>> {
    Ok(AmeData::<System> {
        label: old.label,
        net: migrate_field!(ctx, old.net),
    })
}

#[backends(all)]
fn a_nested_step_reading_its_own_prefix_is_not_a_cycle(backend: Backend) {
    let at = TempPath::new("sys_nested_reach");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let system = v1::System::new_with(&store).unwrap();
        system.label().set(3).unwrap();
        system.net().port().set(8080).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    assert_eq!(
        System::new_with(&store).unwrap().net().label_copy().get(),
        3,
        "{backend:?}"
    );
}
