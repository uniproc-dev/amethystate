//! An ordinary program, with `amethystate::prelude::*` and nothing else from
//! this crate.
//!
//! Whether it compiles is the assertion. Anything a program of this shape
//! reaches for and cannot name is missing from the prelude.

use amethystate::prelude::*;
use amethystate_core::test_utils::TempPath;

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

mod was {
    use super::*;

    #[amethystate(prefix = "editor", version = 0)]
    pub struct Editor {
        #[amestate(nested)]
        pub window: Window,

        #[amestate(default = 14u32)]
        pub font_size: u32,
    }
}

#[amethystate(prefix = "editor", rename_all = "camelCase", version = 1)]
pub struct Editor {
    #[amestate(nested)]
    pub window: Window,

    #[amestate(path = "font.size", default = 14u32)]
    pub size: u32,

    #[amestate(default = {}, on_delete = UseDefault)]
    pub open_files: ReactiveMap<String, String>,

    #[amestate(default = None, on_unreadable = UseDefault)]
    pub last_project: Option<String>,
}

#[migrate]
#[rename(font_size => size)]
fn the_font_size_moved(old: AmeData<was::Editor>) -> MigrationResult<AmeData<Editor>> {
    Ok(AmeData::<Editor> {
        window: old.window,
        size: old.font_size,
        open_files: Default::default(),
        last_project: None,
    })
}

fn open_it(at: &TempPath) -> anyhow::Result<(Store, Editor)> {
    let store = StoreBuilder::new(at.path())
        .backend(Backend::Json)
        .rules(|r| r.on_unreadable(OnUnreadable::UseDefault))
        .build()?;

    let editor = Editor::new_with(&store)?;

    Ok((store, editor))
}

fn watch(editor: &Editor) -> ReactiveScope {
    let mut scope = ReactiveScope::new();
    scope.watch(editor.size.subscribe(|_size: &u32| {}));
    scope.watch(editor.open_files.subscribe_any(|change: &MapChange<_, _>| {
        let _ = change;
    }));
    scope
}

fn reach_around(store: &Store, at: &StorePath) -> anyhow::Result<Option<u32>> {
    store.set(at, &1280u32)?;
    store.save_now()?;
    Ok(store.get::<u32>(at)?)
}

fn explain(why: &impl std::fmt::Debug) -> String {
    format!("{why:?}")
}

#[test]
fn a_program_written_against_the_prelude_alone() {
    let at = TempPath::new("prelude");
    let (store, editor) = open_it(&at).unwrap();
    let _scope = watch(&editor);

    let path = StorePath::from_segments(["editor", "fontSize"]);
    assert_eq!(reach_around(&store, &path).unwrap(), Some(1280));

    let whole: Editor = Editor::load_slice(&store).unwrap();
    assert_eq!(whole.window.width.get(), 800);

    let field: Field<u32> = editor.size.clone();
    assert_eq!(field.get(), 14);

    let broken = StoreBuilder::new("").build().unwrap_err();
    assert!(!explain(&broken).is_empty());
}
