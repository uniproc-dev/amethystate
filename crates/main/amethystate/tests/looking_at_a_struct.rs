mod common;

use amethystate::observability::{Inspect, InspectExt, Reason};
use amethystate::prelude::*;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

/// A type nobody can print, to prove a field holding one is still listed.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Unprintable(pub u8);

#[amethystate(prefix = "editor", rename_all = "camelCase")]
pub struct Editor {
    #[amestate(nested)]
    pub window: Window,

    /// How large the text is drawn.
    /// Applies at once, without a restart.
    #[amestate(path = "font.size", default = 14u32)]
    pub font_size: u32,

    #[amestate(default = {})]
    pub open_files: ReactiveMap<String, String>,

    #[amestate(volatile, default = 3u8)]
    pub redraws: u8,

    #[amestate(default = Unprintable(1))]
    pub opaque: Unprintable,

    #[amestate(default = "light".to_string())]
    pub theme: String,
}

fn opened(backend: Backend, at: &TempPath) -> Store {
    StoreBuilder::new(at.path())
        .backend(backend)
        .rules(|r| r.on_unreadable(OnUnreadable::UseDefault))
        .build()
        .unwrap()
}

#[backends(all)]
fn every_declared_field_is_listed_in_order(backend: Backend) {
    let at = TempPath::new("looking_order");
    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    let declared: Vec<&str> = editor.fields().map(|f| f.declared).collect();

    assert_eq!(
        declared,
        [
            "window",
            "font_size",
            "open_files",
            "redraws",
            "opaque",
            "theme"
        ],
        "declaration order, and volatile is not left out"
    );
    assert_eq!(editor.fields().len(), 6, "and the count agrees up front");
}

#[backends(all)]
fn a_view_says_both_names_and_the_whole_path(backend: Backend) {
    let at = TempPath::new("looking_names");
    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    let size = editor.field("font_size").unwrap();
    assert_eq!(size.declared, "font_size", "what the code calls it");
    assert_eq!(size.stored, "font.size", "what the file calls it");
    assert_eq!(size.at.to_string(), "editor.font.size", "and where it is");

    let files = editor.field("open_files").unwrap();
    assert_eq!(
        files.stored, "openFiles",
        "`rename_all` reaches the ones that did not name themselves"
    );
}

#[backends(all)]
fn a_value_is_shown_when_it_can_be(backend: Backend) {
    let at = TempPath::new("looking_values");
    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    editor.font_size.set(18).unwrap();

    assert_eq!(editor.field("font_size").unwrap().shown, "18");

    assert_eq!(
        editor.field("opaque").unwrap().shown,
        "<opaque>",
        "a type that cannot be printed is still listed, and says so"
    );

    assert_eq!(
        editor.field("open_files").unwrap().shown,
        "0 entries",
        "a map says how many it holds rather than printing them"
    );
}

#[backends(all)]
fn a_nested_field_carries_the_struct_under_it(backend: Backend) {
    let at = TempPath::new("looking_nested");
    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    let window = editor.field("window").unwrap();
    let inside = window.inside.expect("a nested field has fields under it");

    assert_eq!(inside.field_count(), 1);
    assert_eq!(
        inside.field_at(0).unwrap().at.to_string(),
        "editor.window.width",
        "and they know where they are, not only what they are called"
    );

    assert!(
        editor.field("font_size").unwrap().inside.is_none(),
        "a leaf has nothing under it"
    );
}

#[backends(all)]
fn what_the_store_disagrees_with_is_on_the_field_it_is_about(backend: Backend) {
    let at = TempPath::new("looking_disagreement");

    {
        let store = opened(backend, &at);
        store.set(["editor", "font", "size"], &"large").unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    let size = editor.field("font_size").unwrap();
    let gone = size.disagreement.expect("a word is not a number");
    assert!(matches!(gone.reason, Reason::WillNotRead(_)));
    assert_eq!(gone.at.to_string(), "editor.font.size");

    assert!(
        editor.field("window").unwrap().disagreement.is_none(),
        "and nothing is said about the fields that read fine"
    );
}

#[backends(all)]
fn a_doc_comment_arrives_as_a_description(backend: Backend) {
    let at = TempPath::new("looking_described");
    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    assert_eq!(
        editor.field("font_size").unwrap().described,
        "How large the text is drawn.\nApplies at once, without a restart.",
        "every line of it, joined, without the leading space rustdoc adds"
    );

    assert_eq!(
        editor.field("open_files").unwrap().described,
        "",
        "and nothing where nothing was written"
    );
}

#[backends(all)]
fn the_whole_thing_renders_for_a_person(backend: Backend) {
    let at = TempPath::new("looking_rendered");
    let editor = Editor::new_with(&opened(backend, &at)).unwrap();

    let shown = editor.inspect().to_string();

    assert!(shown.contains("font_size"), "the name in the source");
    assert!(shown.contains("editor.font.size"), "and where it lives");
    assert!(shown.contains("u32"), "and what it was declared as");
    assert!(
        shown.contains("  width"),
        "a nested struct is indented under the field holding it: {shown}"
    );
}
