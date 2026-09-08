mod common;

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{OnDelete, OnUnreadable};
use amethystate::{AmeStateSlice, Store, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

macro_rules! declare {
    ($name:ident, $prefix:literal $(, $struct_rule:ident = $struct_value:ident)?; $($field_rule:ident = $field_value:ident)?) => {
        #[amethystate(prefix = $prefix $(, $struct_rule = $struct_value)?)]
        pub struct $name {
            #[amestate(default = 1280u32 $(, $field_rule = $field_value)?)]
            pub width: u32,
        }
    };
}

declare!(NoneNone, "u_none_none";);
declare!(NoneRefuse, "u_none_refuse", on_unreadable = Refuse;);
declare!(NoneDefault, "u_none_default", on_unreadable = UseDefault;);
declare!(RefuseNone, "u_refuse_none"; on_unreadable = Refuse);
declare!(RefuseRefuse, "u_refuse_refuse", on_unreadable = Refuse; on_unreadable = Refuse);
declare!(RefuseDefault, "u_refuse_default", on_unreadable = UseDefault; on_unreadable = Refuse);
declare!(DefaultNone, "u_default_none"; on_unreadable = UseDefault);
declare!(DefaultDefault, "u_default_default", on_unreadable = UseDefault; on_unreadable = UseDefault);

declare!(DNoneNone, "d_none_none";);
declare!(DNoneKeep, "d_none_keep", on_delete = Keep;);
declare!(DNoneDefault, "d_none_default", on_delete = UseDefault;);
declare!(DKeepNone, "d_keep_none"; on_delete = Keep);
declare!(DKeepKeep, "d_keep_keep", on_delete = Keep; on_delete = Keep);
declare!(DKeepDefault, "d_keep_default", on_delete = UseDefault; on_delete = Keep);
declare!(DDefaultNone, "d_default_none"; on_delete = UseDefault);
declare!(DDefaultKeep, "d_default_keep", on_delete = Keep; on_delete = UseDefault);
declare!(DDefaultDefault, "d_default_default", on_delete = UseDefault; on_delete = UseDefault);

/// One row: what the field said, what the struct said, and what should win.
struct Row {
    at: &'static str,
    said: &'static str,
    opens: fn(&Store) -> bool,
    wins: OnUnreadable,
}

fn row<T: AmeStateSlice>(at: &'static str, said: &'static str, wins: OnUnreadable) -> Row {
    Row {
        at,
        said,
        opens: |store| T::load_slice(store).is_ok(),
        wins,
    }
}

/// The whole matrix, minus the one cell the compiler will not allow: a field
/// asking for `UseDefault` under a struct that promised `Refuse`, which is
/// `field_loosens_the_struct_rule.rs` among the compile-fail cases.
fn unreadable_matrix() -> Vec<Row> {
    use OnUnreadable::{Refuse, UseDefault};

    vec![
        row::<NoneRefuse>("u_none_refuse", "field -, struct Refuse", Refuse),
        row::<NoneDefault>("u_none_default", "field -, struct UseDefault", UseDefault),
        row::<RefuseNone>("u_refuse_none", "field Refuse, struct -", Refuse),
        row::<RefuseRefuse>("u_refuse_refuse", "field Refuse, struct Refuse", Refuse),
        row::<RefuseDefault>(
            "u_refuse_default",
            "field Refuse, struct UseDefault",
            Refuse,
        ),
        row::<DefaultNone>("u_default_none", "field UseDefault, struct -", UseDefault),
        row::<DefaultDefault>(
            "u_default_default",
            "field UseDefault, struct UseDefault",
            UseDefault,
        ),
    ]
}

fn a_word_where_a_number_goes(backend: Backend, path: &TempPath, at: &[&str]) {
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    for prefix in at {
        store.set([*prefix, "width"], &"wide").unwrap();
    }
    store.save_now().unwrap();
    store.close().unwrap();
}

fn opened_with(backend: Backend, path: &TempPath, store_says: OnUnreadable) -> Store {
    StoreBuilder::new(path.path())
        .backend(backend)
        .rules(|r| r.on_unreadable(store_says))
        .build()
        .unwrap()
}

#[backends(all)]
fn what_a_declaration_said_wins_over_what_the_store_says(backend: Backend) {
    let matrix = unreadable_matrix();
    let path = TempPath::new("matrix_unreadable");
    let places: Vec<&str> = matrix.iter().map(|row| row.at).collect();
    a_word_where_a_number_goes(backend, &path, &places);

    for store_says in [OnUnreadable::Refuse, OnUnreadable::UseDefault] {
        let store = opened_with(backend, &path, store_says);

        for row in &matrix {
            let opened = (row.opens)(&store);
            let expected = row.wins == OnUnreadable::UseDefault;

            assert_eq!(
                opened, expected,
                "{}, store {store_says:?}: expected {:?} to win",
                row.said, row.wins
            );
        }
    }
}

#[backends(all)]
fn where_nobody_declared_the_store_decides(backend: Backend) {
    let path = TempPath::new("matrix_unreadable_undeclared");
    a_word_where_a_number_goes(backend, &path, &["u_none_none"]);

    assert!(
        NoneNone::load_slice(&opened_with(backend, &path, OnUnreadable::Refuse)).is_err(),
        "nothing was declared, so `Refuse` on the store is what happens"
    );
    assert!(
        NoneNone::load_slice(&opened_with(backend, &path, OnUnreadable::UseDefault)).is_ok(),
        "and `UseDefault` on the store is what happens instead"
    );
}

#[backends(all)]
fn the_store_answers_only_where_nothing_was_declared(backend: Backend) {
    let path = TempPath::new("matrix_unreadable_default_store");
    let places: Vec<&str> = unreadable_matrix().iter().map(|row| row.at).collect();
    a_word_where_a_number_goes(backend, &path, &places);

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    for row in unreadable_matrix() {
        assert_eq!(
            (row.opens)(&store),
            row.wins == OnUnreadable::UseDefault,
            "{}, store left alone: expected {:?} to win",
            row.said,
            row.wins
        );
    }
}

/// One row of the deletion matrix.
///
/// The whole scenario is the row's own, because telling `Keep` from
/// `UseDefault` needs the handle that was watching when the key went - a fresh
/// one would find nothing stored either way and report the default.
struct Gone {
    said: &'static str,
    run: fn(&Store) -> u32,
    wins: OnDelete,
}

fn watched<T: AmeStateSlice>(store: &Store, at: &str, read: impl FnOnce(&T) -> u32) -> u32 {
    let held = T::load_slice(store).unwrap();
    store.set([at, "width"], &1920u32).unwrap();
    store.save_now().unwrap();
    store
        .delete(amethystate::store::to_path([at, "width"]).unwrap())
        .unwrap();
    read(&held)
}

fn delete_matrix() -> Vec<Gone> {
    use OnDelete::{Keep, UseDefault};

    vec![
        Gone {
            said: "field -, struct Keep",
            wins: Keep,
            run: |s| watched::<DNoneKeep>(s, "d_none_keep", |h| h.width.get()),
        },
        Gone {
            said: "field -, struct UseDefault",
            wins: UseDefault,
            run: |s| watched::<DNoneDefault>(s, "d_none_default", |h| h.width.get()),
        },
        Gone {
            said: "field Keep, struct -",
            wins: Keep,
            run: |s| watched::<DKeepNone>(s, "d_keep_none", |h| h.width.get()),
        },
        Gone {
            said: "field Keep, struct Keep",
            wins: Keep,
            run: |s| watched::<DKeepKeep>(s, "d_keep_keep", |h| h.width.get()),
        },
        Gone {
            said: "field Keep, struct UseDefault",
            wins: Keep,
            run: |s| watched::<DKeepDefault>(s, "d_keep_default", |h| h.width.get()),
        },
        Gone {
            said: "field UseDefault, struct -",
            wins: UseDefault,
            run: |s| watched::<DDefaultNone>(s, "d_default_none", |h| h.width.get()),
        },
        Gone {
            said: "field UseDefault, struct Keep",
            wins: UseDefault,
            run: |s| watched::<DDefaultKeep>(s, "d_default_keep", |h| h.width.get()),
        },
        Gone {
            said: "field UseDefault, struct UseDefault",
            wins: UseDefault,
            run: |s| watched::<DDefaultDefault>(s, "d_default_default", |h| h.width.get()),
        },
    ]
}

#[backends(all)]
fn a_deleted_key_follows_the_same_order(backend: Backend) {
    for store_says in [OnDelete::Keep, OnDelete::UseDefault] {
        let path = TempPath::new("matrix_delete");
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .rules(|r| r.on_delete(store_says))
            .build()
            .unwrap();

        for row in delete_matrix() {
            let held = (row.run)(&store);
            let expected = match row.wins {
                OnDelete::Keep => 1920,
                OnDelete::UseDefault => 1280,
            };

            assert_eq!(
                held, expected,
                "{}, store {store_says:?}: expected {:?} to win",
                row.said, row.wins
            );
        }

        store.close().unwrap();
    }
}

#[backends(all)]
fn a_deleted_key_where_nobody_declared_is_the_store_word(backend: Backend) {
    for (store_says, expected) in [(OnDelete::Keep, 1920u32), (OnDelete::UseDefault, 1280)] {
        let path = TempPath::new("matrix_delete_undeclared");
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .rules(|r| r.on_delete(store_says))
            .build()
            .unwrap();

        let held = watched::<DNoneNone>(&store, "d_none_none", |h| h.width.get());
        assert_eq!(held, expected, "store {store_says:?} and nothing declared");

        store.close().unwrap();
    }
}

#[backends(all)]
fn a_nested_struct_that_declared_nothing_takes_the_store_word(backend: Backend) {
    let path = TempPath::new("matrix_nested");
    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["deep", "child", "width"], &"wide").unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    assert!(
        Deep::load_slice(&opened_with(backend, &path, OnUnreadable::Refuse)).is_err(),
        "the child declared nothing and neither did its holder, so the store's word reaches it"
    );
    assert!(
        Deep::load_slice(&opened_with(backend, &path, OnUnreadable::UseDefault)).is_ok(),
        "and it reaches it the other way too"
    );
}

//@show one process, two answers about the same store
#[amethystate(prefix = "thumbnails")]
pub struct Thumbnails {
    #[amestate(default = 0u32)]
    pub generated: u32,
}

#[amethystate(prefix = "licence", on_unreadable = Refuse)]
pub struct Licence {
    #[amestate(default = "".to_string())]
    pub holder: String,
}
//@show-end

/// A store opened as a cache, holding one thing that is not.
///
/// Both are in the same process and over the same file. `Thumbnails` declared
/// nothing, so it takes the word the store was opened with and starts anyway;
/// `Licence` promised `Refuse` for itself, and no word from out here loosens
/// it.
#[backends(all)]
fn a_cache_that_still_holds_one_thing_worth_refusing(backend: Backend) {
    let path = TempPath::new("matrix_cache");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["thumbnails", "generated"], &"lots").unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .rules(|r| r.on_unreadable(OnUnreadable::UseDefault))
        .build()
        .unwrap();

    let thumbnails = Thumbnails::new_with(&store).expect("a cache starts over rather than failing");
    assert_eq!(thumbnails.generated.get(), 0);

    Licence::new_with(&store).expect("nothing is wrong with the licence yet");

    store.set(["licence", "holder"], &7u32).unwrap();
    store.save_now().unwrap();

    Licence::new_with(&store)
        .expect_err("and when something is, its own `Refuse` still stops the process");
}

#[amethystate]
pub struct Child {
    #[amestate(default = 1280u32)]
    pub width: u32,
}

#[amethystate(prefix = "deep")]
pub struct Deep {
    #[amestate(nested)]
    pub child: Child,
}

#[backends(all)]
fn the_word_survives_an_open_that_migrates(backend: Backend) {
    let path = TempPath::new("matrix_migrating");
    a_word_where_a_number_goes(backend, &path, &["u_none_none"]);

    let (store, _report) = StoreBuilder::new(path.path())
        .backend(backend)
        .rules(|r| r.on_unreadable(OnUnreadable::UseDefault))
        .build_with_migration()
        .unwrap();

    assert!(
        NoneNone::load_slice(&store).is_ok(),
        "the other way in carries the same word"
    );
}
