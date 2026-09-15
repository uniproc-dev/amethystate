use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
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

mod v1 {
    use super::*;

    #[amethystate(prefix = "clock", version = 1)]
    pub struct Clock {
        #[amestate(with = since_the_epoch, default = UNIX_EPOCH)]
        pub opened: SystemTime,
    }
}

#[amethystate(prefix = "clock", version = 2)]
pub struct Clock {
    #[amestate(with = since_the_epoch, default = UNIX_EPOCH)]
    pub started: SystemTime,
}

#[migrate]
#[rename(opened => started)]
fn migrate_clock_v1_to_v2(old: AmeData<v1::Clock>) -> amethystate::MigrationResult<AmeData<Clock>> {
    Ok(AmeData::<Clock> {
        started: old.opened,
    })
}

#[backends(all)]
fn a_step_reads_and_writes_a_declared_with_in_its_own_form(backend: Backend) {
    let at = TempPath::new("migrated_with");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        v1::Clock::new_with(&store).unwrap();
        store.set(["clock", "opened"], &1_700_000_000u64).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures(), "{report:?}");

    assert_eq!(
        store.get::<u64>(["clock", "started"]).unwrap(),
        Some(1_700_000_000),
        "the step wrote the field in its type's own form rather than its declared one"
    );

    let clock = Clock::new_with(&store).unwrap();
    assert_eq!(
        clock.started().get(),
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    );
}
