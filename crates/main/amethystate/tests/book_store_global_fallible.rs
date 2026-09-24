#![cfg(feature = "redb")]

use amethystate::InitGlobal;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::path::PathBuf;

struct BackTo(PathBuf);

impl Drop for BackTo {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

#[backends(Redb)]
fn a_global_store_that_will_not_open_is_an_error_the_app_answers(
    _backend: Backend,
) -> anyhow::Result<()> {
    let dir = TempPath::new("book_store_global_fallible");
    let _back = BackTo(std::env::current_dir()?);
    std::env::set_current_dir(dir.path().parent().unwrap())?;
    std::fs::write("./app.redb", b"not a database")?;

    //@show opening the process-wide store without a panic
    let _ame = match StoreBuilder::new("./app.redb").build_global() {
        Ok(guard) => guard,
        Err(InitGlobal::Open(why)) => {
            eprintln!("settings are unavailable: {why}");
            return Ok(());
        }
        Err(InitGlobal::AlreadyInstalled) => unreachable!("opened once, in main"),
    };
    //@show-end

    panic!("a file that is not a database opened as the global store");
}
