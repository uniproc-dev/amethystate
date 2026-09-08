mod common;

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod since_the_epoch {
    use super::{Duration, SystemTime, UNIX_EPOCH};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &SystemTime, into: S) -> Result<S::Ok, S::Error> {
        let seconds = value
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?
            .as_secs();
        into.serialize_u64(seconds)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(from: D) -> Result<SystemTime, D::Error> {
        let seconds = u64::deserialize(from)?;
        UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .ok_or_else(|| serde::de::Error::custom("that many seconds is past the end of time"))
    }
}

#[amethystate(prefix = "session")]
pub struct Session {
    #[amestate(with = since_the_epoch, default = UNIX_EPOCH)]
    pub opened: SystemTime,
}

#[backends(all)]
fn the_pair_decides_what_the_path_holds(backend: Backend) {
    let path = TempPath::new("stored_another_way");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let state = Session::new_with(&store).unwrap();
    state
        .opened
        .set(UNIX_EPOCH + Duration::from_secs(1700000000))
        .unwrap();
    store.save_now().unwrap();

    let held: u64 = store.get(["session", "opened"]).unwrap().unwrap();
    assert_eq!(
        held, 1700000000,
        "one number, where the type's own impl writes a struct of two"
    );
}

#[backends(all)]
fn what_the_pair_wrote_the_pair_reads(backend: Backend) {
    let path = TempPath::new("stored_another_way_reopen");
    let at = UNIX_EPOCH + Duration::from_secs(42);

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let state = Session::new_with(&store).unwrap();
        state.opened.set(at).unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    assert_eq!(Session::new_with(&store).unwrap().opened.get(), at);
}

#[backends(all)]
fn a_path_the_pair_cannot_read_names_itself(backend: Backend) {
    let path = TempPath::new("stored_another_way_plain");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["session", "opened"], &"noon").unwrap();
    store.save_now().unwrap();

    let refused = Session::new_with(&store).expect_err("a word is not a count of seconds");
    let rendered = format!("{refused:?}");

    assert!(
        rendered.contains("session.opened"),
        "the refusal says which path it was reading: {rendered}"
    );
}
