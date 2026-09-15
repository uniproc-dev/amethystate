use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate]
pub struct Fonts {
    #[amestate(default = 14u32)]
    pub size: u32,
}

#[amethystate(prefix = "editor")]
pub struct Editor {
    #[amestate(nested, flatten)]
    pub window: Window,

    #[amestate(nested)]
    pub fonts: Fonts,
}

#[backends(all)]
fn a_flattened_child_gives_up_its_segment(backend: Backend) {
    let path = TempPath::new("a_flattened_child");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let editor = Editor::new_with(&store).unwrap();

    assert_eq!(store.get::<u32>(["editor", "width"]).unwrap(), Some(800));
    assert_eq!(
        store.get::<u32>(["editor", "window", "width"]).unwrap(),
        None
    );
    assert_eq!(
        store.get::<u32>(["editor", "fonts", "size"]).unwrap(),
        Some(14)
    );

    editor.window.width().set(1024).unwrap();
    store.save_now().unwrap();

    assert_eq!(store.get::<u32>(["editor", "width"]).unwrap(), Some(1024));

    store.close().unwrap();
}

#[amethystate(prefix = "deep")]
pub struct Outer {
    #[amestate(nested, flatten)]
    pub middle: Middle,
}

#[amethystate]
pub struct Middle {
    #[amestate(nested, flatten)]
    pub window: Window,
}

#[backends(all)]
fn flattening_passes_through_more_than_one_level(backend: Backend) {
    let path = TempPath::new("a_flattened_grandchild");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let _outer = Outer::new_with(&store).unwrap();

    assert_eq!(store.get::<u32>(["deep", "width"]).unwrap(), Some(800));

    store.close().unwrap();
}

/// A flattened child's field is at `editor.width`, so a raw write there is the
/// declaration's place and has to be refused - the collision walk has to look
/// for it at the level the flatten put it, not under the node's name.
#[backends(all)]
fn a_raw_write_onto_a_flattened_childs_field_is_refused(backend: Backend) {
    let at = TempPath::new("flattened_kv_collision");
    let store = StoreBuilder::new(&at).backend(backend).build().unwrap();
    let _editor = Editor::new_with(&store).unwrap();

    let refused = store.kv().namespace("editor").set("width", &1u32);

    assert!(
        refused.is_err(),
        "`editor.width` is the flattened child's field, and a kv write took it"
    );
}
