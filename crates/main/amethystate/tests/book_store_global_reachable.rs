use amethystate::store::builder::Backend;
use amethystate::{IntoGlobalStore, global_store};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(Redb)]
fn the_global_store_is_reachable_without_being_passed_around(
    _backend: Backend,
) -> anyhow::Result<()> {
    let home = std::env::current_dir()?;
    let dir = TempPath::new("book_store_global_reachable");
    std::env::set_current_dir(dir.path().parent().unwrap())?;

    let _ame = "./app.redb".init_global();

    //@show reaching the global store from anywhere
    let store = global_store();
    store.kv().set("theme", &"dark".to_string())?;
    //@show-end

    std::env::set_current_dir(home)?;

    Ok(())
}
