use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;

#[amethystate(prefix = "net")]
pub struct ConnectionState {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,
}

fn open(
    backend: Backend,
    tag: &str,
) -> Result<(ConnectionState, TempPath), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    Ok((ConnectionState::new_with(&store)?, path))
}

#[backends(all)]
fn the_four_ways_to_touch_a_field(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_fields_ops")?;

    //@show reading and writing a field
    let port = state.port().get();

    state.port().set(9090)?;

    let raised = state.port().update(|port| port + 1)?;

    state.port().modify(|port| *port += 1)?;
    //@show-end

    assert_eq!(port, 8080);
    assert_eq!(raised, 9091);
    assert_eq!(state.port().get(), 9092);

    Ok(())
}

#[backends(all)]
fn update_hands_back_what_it_stored(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_fields_update")?;

    assert_eq!(state.port().update(|port| port * 2)?, 16160);
    assert_eq!(state.port().get(), 16160);

    Ok(())
}

#[backends(all)]
fn a_write_is_visible_to_the_next_read(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_fields_read")?;

    state.host().set("0.0.0.0".to_string())?;
    assert_eq!(state.host().get(), "0.0.0.0");

    Ok(())
}
