use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeType, ReactiveMap};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

//@show declaring a state struct
use amethystate::amethystate;

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
//@show-end

//@show a struct in persistent mode
#[amethystate(prefix = "kept", mode = "persistent")]
pub struct KeptSettings {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
//@show-end

//@show a map whose keys are not known up front
#[derive(Debug, Clone, Serialize, Deserialize, Default, AmeType)]
pub struct AlertThresholds {
    pub warning: u64,
    pub critical: u64,
}

#[amethystate(prefix = "sys")]
pub struct SystemSettings {
    #[amestate(default = {
        "cpu": AlertThresholds { warning: 70, critical: 90 },
        "mem": AlertThresholds { warning: 80, critical: 95 }
    })]
    pub limits: ReactiveMap<String, AlertThresholds>,
}
//@show-end

#[backends(Redb)]
fn opening_a_store_and_reading_a_field(
    _backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_quick_start");
    let settings = path.path();

    //@show opening a store you hold yourself
    let store = StoreBuilder::new(settings)
        .disk(|d| d.debounce(Duration::from_millis(500)))
        .build()?;

    let state = NetworkState::new_with(&store)?;
    //@show-end

    assert_eq!(state.host().get(), "127.0.0.1");
    assert_eq!(state.port().get(), 8080);

    Ok(())
}

#[backends(all)]
fn writing_a_field_and_hearing_about_it(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_quick_start_write");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = NetworkState::new_with(&store)?;

    let heard = Arc::new(AtomicU16::new(0));
    let seen = Arc::clone(&heard);
    let _watching = state
        .port()
        .subscribe(move |port| seen.store(*port, Ordering::Release));

    //@show reading, writing and subscribing
    println!("{}", state.host().get());

    let _sub = state.port().subscribe(|port| {
        println!("port changed to {port}");
    });

    state.port().set(9090)?;
    //@show-end

    assert_eq!(state.port().get(), 9090);
    assert_eq!(heard.load(Ordering::Acquire), 9090);

    Ok(())
}

#[backends(all)]
fn a_persistent_struct_is_plain_fields(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_quick_start_kept");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    //@show writing a persistent struct
    let mut state = KeptSettings::load_with(&store)?;

    state.port = 9090;
    state.save()?;

    state.mutate(|d| {
        d.host = "0.0.0.0".to_string();
        d.port = 443;
    })?;
    //@show-end

    assert_eq!(state.host, "0.0.0.0");
    assert_eq!(state.port, 443);

    Ok(())
}

#[backends(all)]
fn a_map_takes_entries_it_was_not_declared_with(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_quick_start_map");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = SystemSettings::new_with(&store)?;

    //@show working with a map
    state.limits().insert(
        "gpu".to_string(),
        &AlertThresholds {
            warning: 60,
            critical: 85,
        },
    )?;

    let cpu = state.limits().get("cpu");

    for (key, value) in state.limits().entries() {
        println!("{key}: {value:?}");
    }

    let _sub = state.limits().subscribe_any(|change| {
        println!("{change:?}");
    });
    //@show-end

    assert_eq!(cpu.map(|limits| limits.warning), Some(70));
    assert_eq!(state.limits().entries().count(), 3);

    Ok(())
}
