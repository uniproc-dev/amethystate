use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::unique_path;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "hybrid_profile", version = 1)]
    pub struct Profile {
        #[amestate(default = "Ada Lovelace".to_string())]
        pub full_name: String,

        #[amestate(default = true)]
        pub legacy_flag: bool,
    }
}

mod v2 {
    use super::*;

    #[amethystate(prefix = "hybrid_profile", version = 2)]
    pub struct Profile {
        #[amestate(default = "Ada Lovelace".to_string())]
        pub display_name: String,
    }
}

#[amethystate(prefix = "hybrid_profile", version = 3)]
pub struct Profile {
    #[amestate(default = "Ada Lovelace".to_string())]
    pub display_name: String,

    #[amestate(default = "AL".to_string())]
    pub initials: String,
}

#[migrate]
#[rename(full_name => display_name)]
fn migrate_profile_v1_to_v2(
    old: AmeData<v1::Profile>,
) -> amethystate::MigrationResult<AmeData<v2::Profile>> {
    Ok(AmeData::<v2::Profile> {
        display_name: old.full_name,
    })
}

#[backends(all)]
fn migration_builder_mixes_codegen_and_manual_steps(backend: Backend) {
    let path = unique_path("migration-builder");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let profile = v1::Profile::new_with(&store).unwrap();
        profile.full_name().set("Grace Hopper".to_string()).unwrap();
        profile.legacy_flag().set(true).unwrap();
    }

    let store = StoreBuilder::new(&path)
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
            m.for_node::<Profile>()
                .step(3, "derive initials after codegen migration", |ctx| {
                    let display_name = ctx
                        .get::<String>("display_name")?
                        .expect("codegen step should create display_name");
                    let initials = display_name
                        .split_whitespace()
                        .filter_map(|part| part.chars().next())
                        .collect::<String>();
                    ctx.set("initials", &initials)?;
                    Ok(())
                });
        })
        .build()
        .unwrap();

    let profile = Profile::new_with(&store).unwrap();
    assert_eq!(profile.display_name().get(), "Grace Hopper");
    assert_eq!(profile.initials().get(), "GH");

    assert_eq!(
        store
            .get::<String>(["hybrid_profile", "full_name"])
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .get::<bool>(["hybrid_profile", "legacy_flag"])
            .unwrap(),
        None
    );
}
