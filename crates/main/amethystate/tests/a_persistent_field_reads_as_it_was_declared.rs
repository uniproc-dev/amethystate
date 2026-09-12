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

#[amethystate(prefix = "session", mode = "persistent")]
pub struct Session {
    #[amestate(with = since_the_epoch, default = UNIX_EPOCH)]
    pub opened: SystemTime,
}

#[amethystate(prefix = "cfg", mode = "persistent", on_unreadable = UseDefault)]
pub struct Cfg {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[amethystate(prefix = "strict", mode = "persistent")]
pub struct Strict {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[backends(all)]
fn a_declared_with_is_honoured_where_there_is_no_field(backend: Backend) {
    let path = TempPath::new("persistent_with");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["session", "opened"], &1_700_000_000u64).unwrap();

    let held = Session::load_with(&store).unwrap();

    assert_eq!(held.opened, UNIX_EPOCH + Duration::from_secs(1_700_000_000));
}

#[backends(all)]
fn what_a_save_writes_is_what_a_load_reads_back(backend: Backend) {
    let path = TempPath::new("persistent_with_roundtrip");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let mut held = Session::load_with(&store).unwrap();
    held.opened = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    held.save().unwrap();

    let stored = store.get::<u64>(["session", "opened"]).unwrap();
    assert_eq!(
        stored,
        Some(1_700_000_000),
        "the save wrote a shape the declared `with` does not read"
    );

    let again = Session::load_with(&store).unwrap();
    assert_eq!(
        again.opened,
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    );
}

#[backends(all)]
fn a_value_that_will_not_read_takes_the_default_where_that_was_asked_for(backend: Backend) {
    let path = TempPath::new("persistent_unreadable");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["cfg", "port"], &"noon".to_string()).unwrap();

    let held = Cfg::load_with(&store).unwrap();

    assert_eq!(held.port, 8080);
}

#[backends(all)]
fn a_value_that_will_not_read_refuses_the_load_by_default(backend: Backend) {
    let path = TempPath::new("persistent_strict");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["strict", "port"], &"noon".to_string()).unwrap();

    assert!(Strict::load_with(&store).is_err());
}
