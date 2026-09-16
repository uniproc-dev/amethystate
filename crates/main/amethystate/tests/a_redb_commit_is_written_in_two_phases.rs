#![cfg(feature = "redb")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::Path;

const CHILD: &str = "AME_TWO_PHASE_CHILD";
const REDB_MAGIC: [u8; 9] = [b'r', b'e', b'd', b'b', 0x1A, 0x0A, 0xA9, 0x0D, 0x0A];
const TWO_PHASE_COMMIT: u8 = 4;

fn write_and_stop(at: &Path) -> ! {
    let store = StoreBuilder::new(at)
        .backend(Backend::Redb)
        .build()
        .unwrap();
    store.set(["a"], &1u32).unwrap();
    store.save_now().unwrap();
    std::process::abort();
}

#[test]
fn the_last_commit_of_a_process_that_stopped_was_written_in_two_phases() {
    if let Ok(child) = std::env::var(CHILD) {
        write_and_stop(Path::new(&child));
    }

    let at = TempPath::new("two_phase_stopped");

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("the_last_commit_of_a_process_that_stopped_was_written_in_two_phases")
        .env(CHILD, at.path())
        .output()
        .expect("spawning the writer failed");
    assert!(!child.status.success(), "the writer was not stopped");

    let bytes = std::fs::read(at.path()).unwrap();
    assert_eq!(bytes[..REDB_MAGIC.len()], REDB_MAGIC, "not a redb file");
    assert_ne!(
        bytes[REDB_MAGIC.len()] & TWO_PHASE_COMMIT,
        0,
        "the commit a stopped process left behind was written in one phase"
    );
}
