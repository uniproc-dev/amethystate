use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use amethystate_core::test_utils::TempPath;
use std::time::Duration;

#[amethystate(prefix = "durable")]
pub struct Settings {
    #[amestate(default = 1)]
    pub port: u16,

    #[amestate(default = 0)]
    pub retries: u8,
}

#[amethystate(prefix = "durable_map")]
pub struct Mapped {
    #[amestate(default = { "cpu": 70 })]
    pub limits: ReactiveMap<String, u8>,
}

#[amethystate(prefix = "durable_vol")]
pub struct Volatile {
    #[amestate(default = 0, volatile)]
    pub scratch: u8,
}

/// A volatile field has no store behind it, so the durable writes have nothing
/// to commit and nothing to wait on. What is checked is that they still take
/// the value and return - not that anything reached a disk, which is the one
/// thing this field never does.
#[test]
fn a_volatile_field_is_already_durable() {
    let at = TempPath::new("durable_volatile");
    let store = StoreBuilder::new(&at).build().unwrap();
    let state = Volatile::new_with(&store).unwrap();

    state.scratch().durable().set(3).unwrap();
    futures::executor::block_on(state.scratch().durable().set_async(4)).unwrap();

    assert_eq!(
        state.scratch().get(),
        4,
        "nothing to commit, so nothing to wait for"
    );
}

#[test]
fn nothing_happens_until_the_future_is_polled() {
    let at = TempPath::new("durable_visible");
    let store = StoreBuilder::new(&at)
        .disk(|d| d.debounce(Duration::from_secs(60)))
        .build()
        .unwrap();
    let state = Settings::new_with(&store).unwrap();

    let retries = state.retries();
    let durable = retries.durable();
    let commit = durable.set_async(7);

    assert_eq!(
        state.retries().get(),
        0,
        "an async fn runs nothing until it is polled, the write included"
    );

    futures::executor::block_on(commit).unwrap();
    assert_eq!(state.retries().get(), 7);
}

/// Only a text backend lets the file be read while the store still holds it,
/// which is what makes the commit observable without closing anything. The
/// flush itself is one code path shared by every backend.
#[cfg(feature = "json")]
mod on_disk {
    use super::*;

    fn contents(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn set_durable_commits_before_it_returns() {
        let path = TempPath::new("durable_blocking");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let state = Settings::new_with(&store).unwrap();

        state.port().durable().set(1).unwrap();

        state.port().set(8080).unwrap();
        let buffered = contents(&path);
        assert!(
            buffered.contains("\"port\": 1") && !buffered.contains("8080"),
            "a plain set leaves it buffered, and the debouncer is a minute away - \
             the file still holds the committed value, got: {buffered}"
        );

        state.port().durable().set(9090).unwrap();
        assert!(
            contents(&path).contains("\"port\": 9090"),
            "the durable write committed before returning, with no close and no timer"
        );
    }

    #[test]
    fn set_durable_async_commits_before_it_resolves() {
        let path = TempPath::new("durable_async");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let state = Settings::new_with(&store).unwrap();

        futures::executor::block_on(state.retries().durable().set_async(7)).unwrap();

        let found = contents(&path);
        assert!(
            found.contains("\"retries\": 7"),
            "resolving the future means the value is on disk, got: {found}"
        );
    }

    #[test]
    fn a_durable_map_write_commits_before_it_returns() {
        let path = TempPath::new("durable_map_set");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let state = Mapped::new_with(&store).unwrap();

        state.limits().durable().insert("gpu".into(), &90).unwrap();

        assert!(
            contents(&path).contains("\"gpu\": 90"),
            "on disk without waiting for the debouncer"
        );
    }

    #[test]
    fn a_durable_removal_commits_before_it_returns() {
        let path = TempPath::new("durable_map_remove");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let state = Mapped::new_with(&store).unwrap();

        state.limits().durable().insert("gpu".into(), &90).unwrap();
        state.limits().durable().remove("gpu").unwrap();

        let found = contents(&path);
        assert!(
            !found.is_empty(),
            "the file was never written, so `gpu` being absent from it says nothing"
        );
        assert!(
            !found.contains("gpu"),
            "the removal is on disk too, not just the write that preceded it, got: {found}"
        );
    }

    #[test]
    fn a_durable_kv_write_commits_before_it_returns() {
        let path = TempPath::new("durable_kv");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let kv = store.kv();

        kv.namespace("scratch")
            .durable()
            .set("answer", &42u8)
            .unwrap();

        assert!(
            contents(&path).contains("42"),
            "Kv writes commit the same way"
        );
    }

    #[test]
    fn a_cell_over_a_field_commits() {
        let path = TempPath::new("durable_cell");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let state = Settings::new_with(&store).unwrap();

        let cell = state.port().cell();
        cell.durable().set(7070).unwrap();

        assert!(
            contents(&path).contains("\"port\": 7070"),
            "the erased cell still knows where to commit"
        );
    }

    #[test]
    fn a_cell_over_a_map_entry_commits() {
        let path = TempPath::new("durable_entry");
        let store = StoreBuilder::new(&path)
            .backend(amethystate::store::builder::Backend::Json)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        let state = Mapped::new_with(&store).unwrap();

        state.limits().insert("cpu".into(), &0).unwrap();
        state
            .limits()
            .entry_cell("cpu".into())
            .durable()
            .set(55)
            .unwrap();

        assert!(
            contents(&path).contains("\"cpu\": 55"),
            "map entries commit too"
        );
    }
}

#[test]
fn an_in_memory_cell_has_nothing_to_commit() {
    let cell = amethystate::ReactiveCell::new(1u8);

    cell.durable().set(2).unwrap();
    futures::executor::block_on(cell.durable().set_async(3)).unwrap();

    assert_eq!(
        cell.get(),
        Some(3),
        "the writes still land, there is just no disk"
    );
}
