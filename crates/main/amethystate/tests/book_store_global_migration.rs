use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(Redb)]
fn the_global_store_can_collect_the_migrate_steps(_backend: Backend) -> anyhow::Result<()> {
    let home = std::env::current_dir()?;
    let dir = TempPath::new("book_store_global_migration");
    std::env::set_current_dir(dir.path().parent().unwrap())?;

    //@show opening it with the migration pass
    let (_ame, report) = StoreBuilder::new("./app.redb").migrate_global()?;

    if report.has_drift() {
        eprintln!("a struct changed without a version bump");
    }
    //@show-end

    std::env::set_current_dir(home)?;

    Ok(())
}
