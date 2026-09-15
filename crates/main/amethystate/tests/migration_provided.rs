//! What a migration step needs from outside the store.
//!
//! A step is collected at link time as a bare `fn`, so it captures nothing.
//! Anything the application has to hand it - the settings it is porting away
//! from, a lookup table, a client - used to have no way in except a global.
//! `StoreBuilder::provide` is that way in, and this is the proof it reaches
//! the step.

use amethystate::migration::ComponentOutcome;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod common;
use common::shape;

/// Stands in for whatever the application knows and the store does not.
struct LegacyDefaults {
    port: u16,
}

mod v1 {
    use super::*;

    #[amethystate(prefix = "provided", version = 1)]
    pub struct Settings {
        #[amestate(default = "localhost".to_string())]
        pub host: String,
    }
}

#[amethystate(prefix = "provided", version = 2)]
pub struct Settings {
    #[amestate(default = "localhost".to_string())]
    pub host: String,

    #[amestate(default = 1u16)]
    pub port: u16,
}

#[migrate]
fn migrate_settings_v1_to_v2(
    old: AmeData<v1::Settings>,
    ctx: &mut amethystate::migration::context::MigrationContext,
) -> amethystate::MigrationResult<AmeData<Settings>> {
    let legacy = ctx.require::<LegacyDefaults>()?;

    Ok(AmeData::<Settings> {
        host: old.host,
        port: legacy.port,
    })
}

/// The value reaches the step, and the step's output is what lands.
#[backends(all)]
fn a_provided_value_reaches_a_migration_step(backend: Backend) {
    let path = TempPath::new("migration_provided");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let _v1 = v1::Settings::new_with(&store).unwrap();
        store.save_now().unwrap();
    }

    // `build_with_migration` rather than `build`: only that one collects the
    // steps `#[migrate]` generated, which is the entry the store's own TODO
    // has open against it.
    let (store, report) = StoreBuilder::new(&path)
        .backend(backend)
        .provide(LegacyDefaults { port: 4321 })
        .build_with_migration()
        .unwrap();
    assert!(
        !report.has_failures(),
        "the migration should have run cleanly: {report:?}"
    );

    let settings = Settings::new_with(&store).unwrap();
    assert_eq!(
        settings.port().get(),
        4321,
        "the migration should have taken the port from the provided value"
    );
}

/// A value that could not cross a thread is still fine to hand over, because
/// migrations do not cross one: they run on whoever opened the store. `Rc`
/// stands in here for the toolkit handles and `RefCell` graphs a GUI actually
/// has - if this stops compiling, `Provided` has grown a `Send` bound back.
#[backends(all)]
fn a_value_that_is_not_send_can_still_be_provided(backend: Backend) {
    use std::cell::RefCell;
    use std::rc::Rc;

    let path = TempPath::new("migration_provided_not_send");
    let seen: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let _v1 = v1::Settings::new_with(&store).unwrap();
        store.save_now().unwrap();
    }

    let (_store, report) = StoreBuilder::new(&path)
        .backend(backend)
        .provide(LegacyDefaults { port: 7 })
        .provide(Rc::clone(&seen))
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures(), "{report:?}");
}

/// And a step asking for something nobody provided says so, naming the type,
/// rather than reading as bad data.
///
/// Pinned as a whole rather than searched for a substring: a wiring mistake is
/// diagnosed by what the report *says*, and a `contains` passes just as
/// happily on a report that names the type and explains nothing.
#[backends(all)]
fn a_step_that_needs_something_nobody_provided_says_which(backend: Backend) {
    let path = TempPath::new("migration_provided_missing");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let _v1 = v1::Settings::new_with(&store).unwrap();
        store.save_now().unwrap();
    }

    let (_store, report) = StoreBuilder::new(&path)
        .backend(backend)
        .build_with_migration()
        .unwrap();

    let failure = report
        .components
        .iter()
        .find_map(|component| match &component.outcome {
            ComponentOutcome::Failed { error } => Some(error),
            _ => None,
        })
        .expect("the step should have failed for want of a provided value");

    insta::assert_snapshot!("migration_wants_a_value_nobody_provided", shape(failure));
}
