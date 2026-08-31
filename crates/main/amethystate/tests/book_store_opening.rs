#[cfg(feature = "json")]
use amethystate::store::builder::Backend;
use amethystate::store::builder::{Layout, StoreBuilder};
use amethystate::store::{StoreBackend, StoreLayout};
use amethystate_core::test_utils::TempPath;
use std::error::Error;

#[test]
fn a_store_opens_at_the_path_it_is_given() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_store_open");
    let settings = path.path();

    //@show opening a store at a path you name
    let store = StoreBuilder::new(settings).build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));

    Ok(())
}

#[cfg(feature = "json")]
#[test]
fn the_engine_names_the_file_when_the_caller_does_not()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let dir = TempPath::new("book_store_extension");
    std::fs::create_dir_all(dir.path())?;
    let config_dir = dir.path();

    //@show letting the engine name the file
    let store = StoreBuilder::new(config_dir.join("settings"))
        .backend(Backend::Json)
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    store.save_now()?;

    assert!(
        dir.path().join("settings.json").exists(),
        "the engine's extension was not appended to a path that had none"
    );

    let _ = std::fs::remove_dir_all(dir.path());
    Ok(())
}

#[cfg(feature = "json")]
#[test]
fn an_extension_the_caller_wrote_is_left_alone() -> Result<(), Box<dyn Error + Send + Sync>> {
    let dir = TempPath::new("book_store_named_ext");
    std::fs::create_dir_all(dir.path())?;
    let config_dir = dir.path();

    //@show naming the file yourself
    let store = StoreBuilder::new(config_dir.join("settings.conf"))
        .backend(Backend::Json)
        .build()?;
    //@show-end

    store.kv().set("port", &8080u16)?;
    store.save_now()?;

    assert!(
        dir.path().join("settings.conf").exists(),
        "a name the caller spelled was renamed because an engine was chosen"
    );

    let _ = std::fs::remove_dir_all(dir.path());
    Ok(())
}

#[test]
#[ignore = "compiled for the book; opening these for real writes into the \
            configuration directory of whoever runs the suite"]
fn the_three_places_a_location_can_name() -> Result<(), Box<dyn Error + Send + Sync>> {
    //@show letting the platform say where the file goes
    let config = StoreBuilder::located(|at| at.app("my-app", "settings"))?.build()?;

    let named = StoreBuilder::located(|at| at.app_under(Layout::App, "my-app", "settings"))?
        .build()?;

    let portable = StoreBuilder::located(|at| at.beside_the_executable("settings"))?.build()?;
    //@show-end

    for store in [config, named, portable] {
        store.save_now()?;
    }

    Ok(())
}

#[test]
fn a_location_is_worked_out_rather_than_spelled() -> Result<(), Box<dyn Error + Send + Sync>> {
    let store = StoreBuilder::located(|at| at.beside_the_executable("settings"))?.build()?;

    store.kv().set("port", &8080u16)?;
    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));

    let left = StoreBackend::files(&store);
    drop(store);

    match left {
        Some(StoreLayout::Single { data }) => {
            let _ = std::fs::remove_file(data);
        }
        Some(StoreLayout::Sidecars {
            data,
            meta,
            data_backup,
            meta_backup,
        }) => {
            for file in [data, meta, data_backup, meta_backup] {
                let _ = std::fs::remove_file(file);
            }
        }
        None => panic!("the store did not say which files it opened"),
    }

    Ok(())
}

#[test]
fn closing_lets_something_else_have_the_file() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_store_closing");
    let store = StoreBuilder::new(path.path()).build()?;
    store.kv().set("port", &8080u16)?;

    //@show closing the store and letting go of the file
    if let Err(report) = store.close() {
        eprintln!("the last writes were not saved: {report:?}");
    }
    //@show-end

    assert!(store.kv().get::<u16>("port").is_err());
    assert!(store.kv().set("port", &9090u16).is_err());

    let elsewhere = StoreBuilder::new(path.path()).build()?;
    assert_eq!(elsewhere.kv().get::<u16>("port")?, Some(8080));

    Ok(())
}

#[test]
fn saving_now_says_whether_the_last_writes_landed() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_store_save_now");
    let store = StoreBuilder::new(path.path()).build()?;

    //@show writing the buffer out and hearing whether it landed
    store.kv().set("port", &8080u16)?;

    if let Err(report) = store.save_now() {
        eprintln!("settings were not saved: {report:?}");
    }
    //@show-end

    assert_eq!(store.kv().get::<u16>("port")?, Some(8080));

    Ok(())
}
