#![cfg(feature = "test-utils")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;
use std::path::Path;

const CHILD: &str = "AME_STOPPED_OPEN_CHILD";
const STOP: &str = "AMETHYSTATE_STOP_THE_OPEN_AFTER";

mod v1 {
    use super::*;

    #[amethystate(prefix = "counted", version = 1)]
    pub struct Counted {
        #[amestate(default = 0u32)]
        pub hits: u32,
    }
}

#[amethystate(prefix = "counted", version = 2)]
pub struct Counted {
    #[amestate(default = 0u32)]
    pub hits: u32,
}

#[migrate]
fn add_ten(old: AmeData<v1::Counted>) -> amethystate::MigrationResult<AmeData<Counted>> {
    Ok(AmeData::<Counted> {
        hits: old.hits + 10,
    })
}

fn open_and_stop(backend: Backend, at: &Path) -> ! {
    let _ = StoreBuilder::new(at).backend(backend).migrate();
    std::process::exit(0);
}

fn stopped_after(written: &str, test_name: &str, backend: Backend) {
    if let Ok(child) = std::env::var(CHILD) {
        open_and_stop(backend, Path::new(&child));
    }

    let at = TempPath::new(test_name);

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        v1::Counted::new_with(&store)
            .unwrap()
            .hits()
            .set(1)
            .unwrap();
        store.save_now().unwrap();
    }

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(test_name)
        .env(CHILD, at.path())
        .env(STOP, written)
        .output()
        .expect("spawning the open failed");

    assert!(
        !child.status.success(),
        "{backend:?}: the open was not stopped after its {written} file"
    );

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrate()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    assert_eq!(
        Counted::new_with(&store).unwrap().hits().get(),
        11,
        "{backend:?}: stopped after its {written} file, the step did not run exactly once"
    );
}

#[cfg(feature = "json")]
#[test]
fn a_json_open_stopped_after_its_metadata_runs_its_step_once() {
    stopped_after(
        "meta",
        "a_json_open_stopped_after_its_metadata_runs_its_step_once",
        Backend::Json,
    );
}

#[cfg(feature = "json")]
#[test]
fn a_json_open_stopped_after_its_data_runs_its_step_once() {
    stopped_after(
        "data",
        "a_json_open_stopped_after_its_data_runs_its_step_once",
        Backend::Json,
    );
}

#[cfg(feature = "toml")]
#[test]
fn a_toml_open_stopped_after_its_metadata_runs_its_step_once() {
    stopped_after(
        "meta",
        "a_toml_open_stopped_after_its_metadata_runs_its_step_once",
        Backend::Toml,
    );
}

#[cfg(feature = "toml")]
#[test]
fn a_toml_open_stopped_after_its_data_runs_its_step_once() {
    stopped_after(
        "data",
        "a_toml_open_stopped_after_its_data_runs_its_step_once",
        Backend::Toml,
    );
}

#[cfg(feature = "ron")]
#[test]
fn a_ron_open_stopped_after_its_metadata_runs_its_step_once() {
    stopped_after(
        "meta",
        "a_ron_open_stopped_after_its_metadata_runs_its_step_once",
        Backend::Ron,
    );
}

#[cfg(feature = "ron")]
#[test]
fn a_ron_open_stopped_after_its_data_runs_its_step_once() {
    stopped_after(
        "data",
        "a_ron_open_stopped_after_its_data_runs_its_step_once",
        Backend::Ron,
    );
}
