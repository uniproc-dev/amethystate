use amethystate::store::builder::Backend;
use amethystate::{IntoGlobalStore, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[backends(Redb)]
fn the_global_store_is_opened_once_and_held_by_a_guard(_backend: Backend) -> anyhow::Result<()> {
    let home = std::env::current_dir()?;
    let dir = TempPath::new("book_store_global_open");
    std::env::set_current_dir(dir.path().parent().unwrap())?;

    //@show opening the process-wide store
    let _ame = "./app.redb".init_global();

    let state = NetworkState::new()?;
    //@show-end

    assert_eq!(state.port().get(), 8080);

    std::env::set_current_dir(home)?;

    Ok(())
}
