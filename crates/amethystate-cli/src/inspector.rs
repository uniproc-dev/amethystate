use crate::report::anyhowed;
use amethystate::StoreConfig;
use amethystate::store::InspectorBackend;
use amethystate::stores::{JsonStore, RedbStore, RonStore, SqliteStore, TomlStore};
use std::path::Path;

pub fn open_inspector(path: &Path) -> anyhow::Result<Box<dyn InspectorBackend>> {
    let config = StoreConfig::new(path);
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => Ok(Box::new(
            TomlStore::open(config, Default::default())
                .map_err(anyhowed)?
                .0,
        )),
        Some("json") => Ok(Box::new(
            JsonStore::open(config, Default::default())
                .map_err(anyhowed)?
                .0,
        )),
        Some("ron") => Ok(Box::new(
            RonStore::open(config, Default::default())
                .map_err(anyhowed)?
                .0,
        )),
        Some("db") => Ok(Box::new(
            SqliteStore::open(config, Default::default())
                .map_err(anyhowed)?
                .0,
        )),
        Some("redb") => Ok(Box::new(
            RedbStore::open(config, Default::default())
                .map_err(anyhowed)?
                .0,
        )),
        ext => Err(anyhow::anyhow!("unsupported extension: {ext:?}")),
    }
}
