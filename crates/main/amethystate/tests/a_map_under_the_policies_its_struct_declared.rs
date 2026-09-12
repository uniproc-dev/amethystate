use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "strict_panel")]
pub struct Strict {
    #[amestate(default = { "cpu": 110u64 })]
    pub widths: ReactiveMap<String, u64>,
}

#[amethystate(prefix = "lenient_panel", unreadable_entries = Skip, on_delete = UseDefault)]
pub struct Lenient {
    #[amestate(default = { "cpu": 110u64 })]
    pub widths: ReactiveMap<String, u64>,
}

fn entry(prefix: &str, name: &str) -> StorePath {
    StorePath::from_segments([prefix, "widths", name])
}

#[backends(all)]
fn a_map_told_to_carry_on_leaves_out_the_entry_it_cannot_read(backend: Backend) {
    let at = TempPath::new("map_policy_unreadable");

    for (prefix, lenient) in [("strict_panel", false), ("lenient_panel", true)] {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();

        match lenient {
            false => {
                Strict::new_with(&store).unwrap();
            }
            true => {
                Lenient::new_with(&store).unwrap();
            }
        }

        store.set(entry(prefix, "gpu"), &120u64).unwrap();
        store
            .set(entry(prefix, "npu"), &"not a number".to_string())
            .unwrap();
        store.save_now().unwrap();
        store.close().unwrap();

        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();

        match lenient {
            false => {
                Strict::new_with(&store).expect_err(&format!(
                    "{backend:?}: a struct that said nothing refuses a map it cannot read whole"
                ));
            }
            true => {
                let held = Lenient::new_with(&store).unwrap_or_else(|why| {
                    panic!("{backend:?}: told to carry on, the map still refused: {why}")
                });

                assert_eq!(
                    held.widths().get("gpu"),
                    Some(120),
                    "{backend:?}: an entry that reads was dropped with the one that does not"
                );
                assert_eq!(
                    held.widths().get("npu"),
                    None,
                    "{backend:?}: the entry that will not read reached the map"
                );
                assert_eq!(
                    held.widths().unreadable_keys(),
                    [entry(prefix, "npu")],
                    "{backend:?}: the map left an entry on disk and cannot say which"
                );
            }
        }

        assert_eq!(
            store.get::<String>(entry(prefix, "npu")).unwrap(),
            Some("not a number".to_string()),
            "{backend:?}: the entry left out of the map was taken off the disk with it"
        );

        store.close().unwrap();
        std::fs::remove_file(at.path()).ok();
        std::fs::remove_file(at.path().with_extension("meta")).ok();
    }
}

#[backends(all)]
fn a_map_told_to_use_defaults_seeds_again_when_its_level_goes(backend: Backend) {
    let at = TempPath::new("map_policy_delete");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();

    let kept = Strict::new_with(&store).unwrap();
    let reseeded = Lenient::new_with(&store).unwrap();

    kept.widths().insert("gpu".into(), &120).unwrap();
    reseeded.widths().insert("gpu".into(), &120).unwrap();

    store
        .delete_prefix(StorePath::from_segments(["strict_panel", "widths"]))
        .unwrap();
    store
        .delete_prefix(StorePath::from_segments(["lenient_panel", "widths"]))
        .unwrap();

    assert_eq!(
        kept.widths().len(),
        0,
        "{backend:?}: the level went and the map that keeps what the store left is not empty"
    );

    assert_eq!(
        reseeded.widths().get("cpu"),
        Some(110),
        "{backend:?}: the declared entries did not come back"
    );
    assert_eq!(
        reseeded.widths().get("gpu"),
        None,
        "{backend:?}: an entry nothing declared came back with the defaults"
    );
}
