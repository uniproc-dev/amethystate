use amethystate::IntoGlobalStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(Redb)]
fn the_global_store_can_collect_the_migrate_steps(_backend: Backend) -> anyhow::Result<()> {
    let home = std::env::current_dir()?;
    let dir = TempPath::new("book_store_global_migration");
    std::env::set_current_dir(dir.path().parent().unwrap())?;

    //@show opening it with the migration pass
    let (report, _ame) = StoreBuilder::new("./app.redb").init_global_with_migration();

    if report.has_failures() {
        eprintln!("a migration step failed; the data was put back");
    }
    if report.has_drift() {
        eprintln!("a struct changed without a version bump");
    }
    //@show-end

    std::env::set_current_dir(home)?;

    Ok(())
}
