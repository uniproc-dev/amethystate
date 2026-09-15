//! A step that stays out of the linker's reach.
//!
//! `#[migrate]` registers through `inventory`, which collects at link time -
//! the step is found rather than named, and an application that would rather
//! hand its migrations over has nothing to hand. `#[migrate(explicit)]` leaves
//! a `const` named for the function instead, and `add_steps` takes it.
//!
//! The pair below is one fixture opened twice: the step is invisible to the
//! collector, and runs when it is passed in.

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "explicit_steps", version = 1)]
    pub struct Settings {
        #[amestate(default = 8080u16)]
        pub port: u16,
    }
}

#[amethystate(prefix = "explicit_steps", version = 2)]
pub struct Settings {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "untouched".to_string())]
    pub host: String,
}

#[migrate(explicit)]
fn settings_v1_to_v2(
    old: AmeData<v1::Settings>,
) -> amethystate::MigrationResult<AmeData<Settings>> {
    Ok(AmeData::<Settings> {
        port: old.port,
        host: "the step ran".to_string(),
    })
}

fn a_store_at_v1(backend: Backend, path: &std::path::Path) {
    let store = StoreBuilder::new(path).backend(backend).build().unwrap();
    let _v1 = v1::Settings::new_with(&store).unwrap();
    store.save_now().unwrap();
}

/// `build_with_migration` is the entry that sweeps the binary for steps, and this
/// one is not in the sweep.
#[backends(all)]
fn an_explicit_step_is_not_collected_from_the_linker(backend: Backend) {
    let path = TempPath::new("migration_explicit_uncollected");
    a_store_at_v1(backend, &path);

    let (store, _report) = StoreBuilder::new(&path)
        .backend(backend)
        .build_with_migration()
        .unwrap();

    let settings = Settings::new_with(&store).unwrap();
    assert_eq!(
        settings.host().get(),
        "untouched",
        "the step was declared `explicit` and should not have been found"
    );
}

/// Named and handed over, it does what any other step does.
#[backends(all)]
fn an_explicit_step_runs_when_it_is_handed_over(backend: Backend) {
    let path = TempPath::new("migration_explicit_handed_over");
    a_store_at_v1(backend, &path);

    let (store, report) = StoreBuilder::new(&path)
        .backend(backend)
        .migrations(|m| {
            m.add_steps(&[SETTINGS_V1_TO_V2]);
        })
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures(), "{report:?}");

    let settings = Settings::new_with(&store).unwrap();
    assert_eq!(
        settings.host().get(),
        "the step ran",
        "the step was passed in and should have run"
    );
    assert_eq!(
        settings.port().get(),
        8080,
        "and carried the old value over"
    );
}
