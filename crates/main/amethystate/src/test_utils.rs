use crate::Store;
use amethystate_core::test_utils::TempPath;

/// A store on a path of its own, and the fixture that takes the files away.
///
/// Both, because the directory lives exactly as long as the binding does: a
/// helper handing back the store alone leaves a file per test behind in the
/// system's temporary directory, where nothing ever collects it.
pub fn unique_store(suffix: &str) -> (Store, TempPath) {
    use crate::store::config::StoreConfig;

    let at = TempPath::new(suffix);

    let store = crate::store::builder::default_backend()
        .open_public(StoreConfig::new(at.path()), Default::default())
        .unwrap()
        .0;

    (store, at)
}
