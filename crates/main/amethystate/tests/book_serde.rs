use amethystate::amethystate;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;
use serde::{Deserialize, Serialize};

#[amethystate(prefix = "bare")]
pub struct Bare {
    pub n: u32,
    pub name: String,
}

#[test]
fn a_field_with_no_annotation_takes_its_types_default() -> anyhow::Result<()> {
    let path = TempPath::new("book_serde_bare");
    let store = StoreBuilder::new(path.path()).build()?;

    let _bare = Bare::new_with(&store)?;

    assert_eq!(store.get::<u32>(["bare", "n"])?, Some(0));
    assert_eq!(store.get::<String>(["bare", "name"])?, Some(String::new()));

    store.close()?;
    Ok(())
}

//@show a struct that says where its fields go
#[amethystate(prefix = "net", rename_all = "camelCase")]
pub struct NetState {
    #[amestate(default = 8080u16)]
    pub listen_port: u16,

    #[amestate(path = "tls.enabled", default = false)]
    pub tls: bool,
}
//@show-end

#[test]
fn serde_names_where_a_field_goes() -> anyhow::Result<()> {
    let path = TempPath::new("book_serde_names");
    let store = StoreBuilder::new(path.path()).build()?;

    let _net = NetState::new_with(&store)?;

    assert_eq!(store.get::<u16>(["net", "listenPort"])?, Some(8080));
    assert_eq!(store.get::<bool>(["net", "tls", "enabled"])?, Some(false));

    store.close()?;
    Ok(())
}

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

//@show a nested struct whose fields sit at their holder's level
#[amethystate(prefix = "editor")]
pub struct Editor {
    #[amestate(nested, flatten)]
    pub window: Window,
}
//@show-end

#[test]
fn a_flattened_child_writes_at_its_holders_level() -> anyhow::Result<()> {
    let path = TempPath::new("book_serde_flatten");
    let store = StoreBuilder::new(path.path()).build()?;

    let _editor = Editor::new_with(&store)?;

    assert_eq!(store.get::<u32>(["editor", "width"])?, Some(800));
    assert_eq!(store.get::<u32>(["editor", "window", "width"])?, None);

    store.close()?;
    Ok(())
}

//@show a leaf, where serde answers to nobody here
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Endpoint {
    pub host_name: String,

    #[serde(default)]
    pub port: u16,
}

#[amethystate(prefix = "svc")]
pub struct Service {
    #[amestate(default = Endpoint { host_name: "localhost".to_string(), port: 443 })]
    pub upstream: Endpoint,
}
//@show-end

#[test]
fn a_leaf_keeps_its_own_serde() -> anyhow::Result<()> {
    let path = TempPath::new("book_serde_leaf");
    let store = StoreBuilder::new(path.path()).build()?;

    let _svc = Service::new_with(&store)?;

    assert_eq!(
        store.get::<Endpoint>(["svc", "upstream"])?,
        Some(Endpoint {
            host_name: "localhost".to_string(),
            port: 443,
        }),
        "the whole value sits at one path, encoded the way its own serde says"
    );

    store.close()?;
    Ok(())
}
