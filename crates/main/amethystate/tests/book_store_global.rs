use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{IntoGlobalStore, amethystate, global_store};
use amethystate_test_macros::backends;
use std::error::Error;

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[backends(Redb)]
#[ignore = "compiled for the book; the process-wide store can be installed once \
            per process, so running this decides it for every other test"]
fn the_global_store_is_opened_once_and_held_by_a_guard(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    //@show opening the process-wide store
    let _ame = "./app.redb".init_global();

    let state = NetworkState::new()?;
    //@show-end

    assert_eq!(state.port().get(), 8080);

    Ok(())
}

#[backends(Redb)]
#[ignore = "compiled for the book; installs the process-wide store"]
fn the_global_store_is_reachable_without_being_passed_around(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let _ame = "./app.redb".init_global();

    //@show reaching the global store from anywhere
    let store = global_store();
    store.kv().set("theme", &"dark".to_string())?;
    //@show-end

    Ok(())
}

#[backends(Redb)]
#[ignore = "compiled for the book; installs the process-wide store"]
fn the_global_store_can_collect_the_migrate_steps(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    //@show opening it with the migration pass
    let (report, _ame) = StoreBuilder::new("./app.redb").init_global_with_migration();

    if report.has_failures() {
        eprintln!("a migration step failed; the data was put back");
    }
    if report.has_drift() {
        eprintln!("a struct changed without a version bump");
    }
    //@show-end

    Ok(())
}
