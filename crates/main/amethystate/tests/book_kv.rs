use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod common;

#[amethystate(prefix = "network")]
pub struct Network {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

fn open(backend: Backend, tag: &str) -> anyhow::Result<(TempPath, amethystate::Store)> {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    Ok((path, store))
}

#[backends(all)]
fn raw_values_at_paths(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_raw")?;

    //@show reading and writing without a schema
    let kv = store.kv();

    kv.set("theme", &"dark".to_string())?;
    let theme = kv.get::<String>("theme")?;

    let ui = kv.namespace("ui");
    ui.set("width", &800u32)?;
    let under_ui = ui.keys()?;

    kv.remove("theme")?;
    //@show-end

    assert_eq!(theme.as_deref(), Some("dark"));
    assert_eq!(kv.get::<String>("theme")?, None);

    let listed: Vec<String> = under_ui.iter().map(|key| key.to_string()).collect();
    assert_eq!(
        listed,
        ["ui.width"],
        "keys are the whole path, not the leaf"
    );

    Ok(())
}

#[backends(all)]
fn what_a_listing_covers(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_keys")?;
    let kv = store.kv();

    //@show what a listing covers
    kv.set("theme", &"dark".to_string())?;
    kv.namespace("ui").set("width", &800u32)?;
    kv.namespace("ui").namespace("panel").set("left", &true)?;

    let from_ui: Vec<String> = kv
        .namespace("ui")
        .keys()?
        .iter()
        .map(|key| key.to_string())
        .collect();

    assert_eq!(from_ui, ["ui.panel.left", "ui.width"]);
    //@show-end

    let from_root: Vec<String> = kv.keys()?.iter().map(|key| key.to_string()).collect();

    assert!(
        from_root.contains(&"theme".to_string()) && from_root.contains(&"ui.width".to_string()),
        "the root handle lists the whole store: {from_root:?}"
    );

    Ok(())
}

#[backends(all)]
fn a_cell_and_a_map_without_a_struct(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_primitives")?;
    let kv = store.kv();

    //@show a cell and a map with nothing declared
    let width = kv.namespace("ui").cell("width", 800u32)?;
    let flags = kv.map::<String, bool>("flags")?;

    flags.insert("dark".to_string(), &true)?;
    //@show-end

    assert_eq!(width.get(), Some(800));
    assert_eq!(flags.get("dark"), Some(true));

    Ok(())
}

#[backends(all)]
fn a_path_a_struct_declared_is_refused(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_owned")?;
    let _network = Network::new_with(&store)?;
    let kv = store.kv();

    //@show writing where a struct lives
    let refused = kv.namespace("network").set("port", &"8080".to_string());

    kv.namespace("networkish")
        .set("port", &"8080".to_string())?;
    //@show-end

    assert!(refused.is_err(), "the declared prefix is not Kv's to write");

    Ok(())
}

#[backends(all)]
fn the_stored_value_decides_the_type(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_types")?;
    let kv = store.kv();

    //@show asking for one path as two types
    let ui = kv.namespace("ui");

    let _width = ui.cell("width", 800u32)?;
    let refused = ui.cell("width", String::new());
    //@show-end

    assert!(refused.is_err(), "800 does not read back as a String");

    Ok(())
}

#[backends(all)]
fn a_cell_fills_the_path_it_was_asked_about(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_empty")?;
    let kv = store.kv();
    let empty = kv.namespace("never_written");

    assert_eq!(kv.namespace("never_written").keys()?.len(), 0);

    let _as_number = empty.cell("thing", 0u32)?;

    assert!(
        empty.cell("thing", String::new()).is_err(),
        "the first cell seeded the path, so the second one finds a number there"
    );

    Ok(())
}

#[backends(all)]
fn raw_writes_are_checked_by_nothing(backend: Backend) -> anyhow::Result<()> {
    let (_path, store) = open(backend, "book_kv_raw_types")?;
    let kv = store.kv();

    kv.set("thing", &1u32)?;
    kv.set("thing", &"now a string".to_string())?;

    assert_eq!(
        kv.get::<String>("thing")?.as_deref(),
        Some("now a string"),
        "a raw write replaces whatever was there, whatever its type"
    );
    assert!(
        kv.get::<u32>("thing").is_err(),
        "asking for the old type fails at the read"
    );

    Ok(())
}

#[amethystate(prefix = "chrome")]
pub struct Chrome {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[cfg(feature = "json")]
#[backends(Json)]
fn what_a_kv_write_looks_like_in_the_file(backend: Backend) -> anyhow::Result<()> {
    let (path, store) = open(backend, "book_kv_on_disk")?;
    let _chrome = Chrome::new_with(&store)?;

    //@show a declared struct and a Kv write, side by side in one file
    store
        .kv()
        .namespace("plugins")
        .namespace("left")
        .set("width", &240u32)?;
    //@show-end

    store.save_now()?;
    let written = std::fs::read_to_string(path.path())?;

    assert!(
        written.contains(r#""plugins.left.width""#),
        "a path nothing declares is one whole key: {written}"
    );
    assert!(
        written.contains(r#""chrome""#) && written.contains(r#""width""#),
        "and a declared struct still nests: {written}"
    );

    common::measured(&[
        ("what failed", "nothing"),
        ("the file a Kv write leaves", written.trim()),
    ]);

    Ok(())
}
