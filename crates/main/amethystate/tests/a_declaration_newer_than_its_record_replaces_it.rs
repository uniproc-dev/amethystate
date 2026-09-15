#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

#[amethystate(prefix = "profile", version = 2)]
pub struct Profile {
    #[amestate(default = String::new())]
    pub name: String,
}

fn recorded(meta: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(meta).unwrap()).unwrap()
}

#[test]
fn a_declaration_newer_than_its_record_replaces_it() {
    let at = TempPath::new("newer_than_record");
    let meta = at.path().with_extension("meta");

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let profile = Profile::new_with(&store).unwrap();
        profile.name().set("Ada".to_string()).unwrap();
        store.close().unwrap();
    }

    let mut older = recorded(&meta);
    assert_eq!(
        older["schema.profile"][0]["version"],
        serde_json::json!(2),
        "{older:#}"
    );
    older["schema.profile"][0]["version"] = serde_json::json!(1);
    std::fs::write(&meta, serde_json::to_string_pretty(&older).unwrap()).unwrap();

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        store.close().unwrap();
    }

    let now = recorded(&meta);
    assert_eq!(
        now["schema.profile"][0]["version"],
        serde_json::json!(2),
        "{now:#}"
    );
}

#[test]
fn a_record_that_disagrees_at_the_same_version_is_replaced() {
    let at = TempPath::new("disagrees_with_record");
    let meta = at.path().with_extension("meta");

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let profile = Profile::new_with(&store).unwrap();
        profile.name().set("Ada".to_string()).unwrap();
        store.close().unwrap();
    }

    let mut other = recorded(&meta);
    assert_eq!(
        other["schema.profile"][0]["fields"][0]["type_name"],
        serde_json::json!("String"),
        "{other:#}"
    );
    other["schema.profile"][0]["fields"][0]["type_name"] = serde_json::json!("u32");
    std::fs::write(&meta, serde_json::to_string_pretty(&other).unwrap()).unwrap();

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        store.close().unwrap();
    }

    let now = recorded(&meta);
    assert_eq!(
        now["schema.profile"][0]["fields"][0]["type_name"],
        serde_json::json!("String"),
        "{now:#}"
    );
}
