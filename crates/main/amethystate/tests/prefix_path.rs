use amethystate::store::StorePath;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{StateScope, amethystate};
use amethystate_test_macros::backends;

#[amethystate(prefix = "ui.window")]
pub struct WindowState {
    #[amestate(default = 1280)]
    pub width: u32,
}

#[amethystate(prefix = "net")]
pub struct NetState {
    #[amestate(default = 8080u16, key = "listen_port")]
    pub port: u16,
}

#[amethystate(as_root)]
pub struct RootState {
    #[amestate(default = 1280)]
    pub width: u32,
}

/// What the compile-time check in `StorePath::from_static` protects, pinned from
/// outside it: the path the attribute writes is the path its levels build, key
/// and all. The two are written by different code - one at expansion time out of
/// string literals, the other at runtime out of the same levels - and a store
/// addresses a value by the second while the macro hands it the first.
#[test]
fn a_prefix_builds_the_path_its_levels_build() {
    let cases = [
        (
            <WindowState as StateScope>::PATH,
            <WindowState as StateScope>::KEY,
            vec!["ui", "window"],
        ),
        (
            <NetState as StateScope>::PATH,
            <NetState as StateScope>::KEY,
            vec!["net"],
        ),
        (
            <RootState as StateScope>::PATH,
            <RootState as StateScope>::KEY,
            vec![],
        ),
    ];

    for (path, key, levels) in cases {
        let built = StorePath::from_segments(&levels);

        assert_eq!(path, built, "levels: {levels:?}");
        assert_eq!(path.as_str(), built.as_str(), "levels: {levels:?}");
        assert_eq!(key, built.as_str(), "levels: {levels:?}");
        assert_eq!(
            path.segments().collect::<Vec<_>>(),
            levels,
            "levels: {levels:?}"
        );
    }
}

/// The separator is what makes a level, so the attribute never grows one out of
/// a name: `ui.window` is two levels and stays two.
#[test]
fn a_written_prefix_keeps_the_levels_it_names() {
    assert_eq!(<WindowState as StateScope>::PATH.len(), 2);
    assert_eq!(<WindowState as StateScope>::KEY, "ui.window");
    assert_ne!(
        <WindowState as StateScope>::KEY,
        StorePath::from_segments(["ui.window"]).as_str(),
        "one level called `ui.window` is a different path"
    );
}

/// A field key names a level under the prefix, and it is checked the same way -
/// which is why a store written by the macro reads back through `StorePath`.
#[backends(all)]
fn a_field_key_names_a_level_under_the_prefix(backend: Backend) {
    let store = StoreBuilder::new(amethystate_core::test_utils::unique_path("prefix_path_key"))
        .backend(backend)
        .build()
        .unwrap();

    let net = NetState::new_with(&store).unwrap();
    net.port().set(9090).unwrap();

    assert_eq!(
        store.get::<u16>(["net", "listen_port"]).unwrap(),
        Some(9090),
        "the key names the level, and the field's own name does not"
    );
    assert_eq!(store.get::<u16>(["net", "port"]).unwrap(), None);
}
