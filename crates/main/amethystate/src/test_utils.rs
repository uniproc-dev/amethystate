use crate::Store;
pub use amethystate_core::test_utils::TempPath;

/// The fixture that takes the files away, and a store on a path of its own.
///
/// Both, because the directory lives exactly as long as the binding does: a
/// helper handing back the store alone leaves a file per test behind in the
/// system's temporary directory, where nothing ever collects it.
///
/// The path comes first because bindings drop in reverse order of the pattern.
/// `let (_at, store) = unique_store(..)` drops the store and then the directory
/// it held; the other way round the directory goes while the store still has
/// its file open, the removal fails, and the fixture's `Drop` cannot say so.
pub fn unique_store(suffix: &str) -> (TempPath, Store) {
    use crate::store::config::StoreConfig;

    let at = TempPath::new(suffix);

    let store = crate::store::builder::built_in()
        .ok_or_else(crate::store::builder::no_engine_built_in)
        .and_then(|backend| backend.open_public(StoreConfig::new(at.path()), Default::default()))
        .unwrap()
        .0;

    (at, store)
}
