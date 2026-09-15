use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "kept_map", mode = "persistent")]
pub struct Kept {
    #[amestate(default = { "base": 0 })]
    pub limits: ReactiveMap<String, u32>,
}

#[backends(all)]
fn an_entry_taken_out_of_a_persistent_map_is_gone_after_a_save(backend: Backend) {
    let at = TempPath::new("persistent_map_removal");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();

        let mut kept = Kept::load_with(&store).unwrap();
        kept.mutate(|now| {
            now.limits.insert("gone".to_string(), 1);
            now.limits.insert("kept".to_string(), 2);
        })
        .unwrap();

        let mut again = Kept::load_with(&store).unwrap();
        again
            .mutate(|now| {
                now.limits.shift_remove("gone");
            })
            .unwrap();

        let reread = Kept::load_with(&store).unwrap();
        assert_eq!(reread.limits.get("gone"), None, "{backend:?}: in the store");
        assert_eq!(
            reread.limits.get("kept"),
            Some(&2),
            "{backend:?}: in the store"
        );
    }

    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();
    let reopened = Kept::load_with(&store).unwrap();

    assert_eq!(reopened.limits.get("gone"), None, "{backend:?}: on disk");
    assert_eq!(
        reopened.limits.get("kept"),
        Some(&2),
        "{backend:?}: on disk"
    );
}
