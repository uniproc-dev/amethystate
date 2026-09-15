use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "panel", unreadable_entries = Skip)]
pub struct Panel {
    #[amestate(default = { "cpu": 110u64 })]
    pub widths: ReactiveMap<String, u64>,
}

fn entry(name: &str) -> StorePath {
    StorePath::from_segments(["panel", "widths", name])
}

#[backends(all)]
fn an_entry_that_will_not_read_is_named_until_something_readable_replaces_it(backend: Backend) {
    let at = TempPath::new("map_unreadable_live");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();
    let panel = Panel::new_with(&store).unwrap();

    let _ = store.set(entry("npu"), &"not a number".to_string());
    let _ = store.set(entry("gpu"), &"not a number either".to_string());

    assert_eq!(
        panel.widths().unreadable_keys(),
        [entry("gpu"), entry("npu")],
        "{backend:?}: arrived and would not read"
    );

    store.set(entry("npu"), &7u64).unwrap();
    store.delete(entry("gpu")).unwrap();

    assert_eq!(
        panel.widths().unreadable_keys(),
        Vec::<StorePath>::new(),
        "{backend:?}: replaced by a readable value and deleted"
    );
    assert_eq!(panel.widths().get("npu"), Some(7), "{backend:?}");
}
