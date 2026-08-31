use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;

mod before {
    use super::*;

    #[amethystate(prefix = "seed")]
    pub struct Cfg {
        #[amestate(default = {"a": 1u64})]
        pub first: ReactiveMap<String, u64>,
    }
}

#[amethystate(prefix = "seed")]
pub struct Cfg {
    #[amestate(default = {"a": 1u64})]
    pub first: ReactiveMap<String, u64>,

    #[amestate(default = {"x": 10u64, "y": 20u64})]
    pub second: ReactiveMap<String, u64>,
}

/// The initialized flag used to be one per scope, set when the struct was first
/// built, so a map added to that struct later never seeded its defaults for
/// anyone already running - it came up empty with nothing to say why.
#[backends(all)]
fn a_map_added_later_still_gets_its_defaults(backend: Backend) {
    let path = unique_path("map_defaults_added");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let old = before::Cfg::new_with(&store).unwrap();
        assert_eq!(old.first().len(), 1);
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    assert_eq!(
        cfg.second().keys().collect::<Vec<_>>(),
        ["x", "y"],
        "the map added in this version must be seeded"
    );
    assert_eq!(cfg.first().keys().collect::<Vec<_>>(), ["a"]);
}

/// Reopening must not undo removals: keys already on disk count as evidence the
/// map was seeded before, so its defaults are not written again. Both halves
/// build the same version - what is checked is the second construction, not a
/// version change.
#[backends(all)]
fn reopening_does_not_restore_a_removed_entry(backend: Backend) {
    let path = unique_path("map_defaults_removed");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        cfg.second().remove("x").unwrap();
        assert_eq!(cfg.second().keys().collect::<Vec<_>>(), ["y"]);
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    assert_eq!(
        cfg.second().keys().collect::<Vec<_>>(),
        ["y"],
        "x stays removed"
    );
}

#[backends(all)]
fn a_fresh_store_seeds_every_map(backend: Backend) {
    let path = unique_path("map_defaults_fresh");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    assert_eq!(cfg.first().keys().collect::<Vec<_>>(), ["a"]);
    assert_eq!(cfg.second().keys().collect::<Vec<_>>(), ["x", "y"]);
}
