use amethystate::store::builder::StoreBuilder;
use amethystate::store::{StoreBackend, StoreLayout};
use amethystate_core::test_utils::TempPath;
use std::error::Error;

mod common;

#[test]
fn a_store_says_which_files_it_opened() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_files_layout");
    let store = StoreBuilder::new(path.path()).build()?;

    //@show asking a store where its files are
    match StoreBackend::files(&store) {
        Some(StoreLayout::Single { data }) => {
            println!("everything is in {}", data.display());
        }
        Some(StoreLayout::Sidecars {
            data,
            meta,
            data_backup,
            meta_backup,
        }) => {
            println!("values:      {}", data.display());
            println!("bookkeeping: {}", meta.display());
            println!("kept while rewriting: {}, {}",
                data_backup.display(),
                meta_backup.display(),
            );
        }
        None => println!("this engine does not say"),
    }
    //@show-end

    assert!(
        StoreBackend::files(&store).is_some(),
        "every engine in this crate names its files"
    );

    Ok(())
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_text_store_keeps_its_bookkeeping_beside_the_data()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_files_text");
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .build()?;

    store.kv().set("port", &8080u16)?;
    store.save_now()?;

    let Some(StoreLayout::Sidecars { data, meta, .. }) = StoreBackend::files(&store) else {
        panic!("a text store keeps two files");
    };

    assert!(data.exists(), "the data file was not written");
    assert!(meta.exists(), "the bookkeeping file was not written");

    Ok(())
}
