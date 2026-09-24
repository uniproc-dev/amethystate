use crate::AmeBackendAsync as AmeBackend;
use crate::facts::Facts;
use crate::failure::StorageError;
use crate::path::{PathRef, StorePath};
use crate::primitives::error::{ReactiveMapError, ReactiveMapResult, WriteValue};
use crate::primitives::map_core::{
    EntryOf, MapEntryPath, ReactiveMapKey, ReactiveMapValue, entry_of,
};
use crate::{MapChange, ReactiveMapCore, map_apply_remote_change};
use uuid::Uuid;

use serde::de::DeserializeOwned;

/// A key under the map that the map cannot take, said as the disk in every
/// sense - which is the only shape this crate's error set has for it.
///
/// The store's own loader answers the same two cases with
/// `LoadMap::{KeyWillNotRead, KeyIsNotAnEntry}`, which name the key and the map
/// separately. Saying it precisely here wants that set in this crate.
fn will_not_take(under: &StorePath, stored: &StorePath, said: &'static str) -> WriteValue {
    WriteValue::Store(
        error_stack::Report::new(StorageError::Read)
            .attach(said)
            .attach(crate::facts::Prefix(under.clone()))
            .attach(crate::facts::Key(stored.clone()))
            .into(),
    )
}

async fn read_entry<B, V>(backend: &B, entry: &StorePath) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackend,
    V: DeserializeOwned,
{
    backend
        .get::<V>(entry)
        .await
        .attach_key(entry)
        .map_err(|why| WriteValue::from_backend(entry, StorageError::Read, why))
}

pub async fn map_get_async<B, K, V>(
    backend: &B,
    path: &StorePath,
    key: &K,
) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackend,
    K: AsRef<str>,
    V: DeserializeOwned,
{
    let entry = path.entry(key.as_ref());
    read_entry::<B, V>(backend, &entry).await
}

/// Every entry stored under `path`, keyed by the level below it.
///
/// What a key is - an entry, the path itself, a name that will not read as `K`,
/// or somebody else's - is [`entry_of`]'s answer rather than this function's, so
/// a map built here and a map built by a store that reads now cannot disagree
/// about a file. This one refuses what it cannot take, which is what
/// `UnreadableEntries::Refuse` means on the other side; there is no way to ask
/// for the other answer here yet.
pub async fn map_entries_async<B, K, V>(
    backend: &B,
    path: &StorePath,
) -> ReactiveMapResult<Vec<(K, V)>>
where
    B: AmeBackend,
    K: ReactiveMapKey,
    V: DeserializeOwned + Default,
{
    let kvs = backend
        .scan_prefix(path)
        .await
        .attach_prefix(path)
        .map_err(|why| WriteValue::from_backend(path, StorageError::Scan, why))?;
    let mut results = Vec::new();

    for (full_path, raw) in kvs {
        let key = match entry_of::<K>(path, PathRef::from(&full_path)) {
            EntryOf::Took(key) => key,
            EntryOf::ThePathItself => continue,
            EntryOf::NameWillNotRead => {
                return Err(will_not_take(
                    path,
                    &full_path,
                    "the name this entry is stored under does not read as the map's key type",
                ));
            }
            EntryOf::NotThisMaps => {
                return Err(will_not_take(
                    path,
                    &full_path,
                    "a map owns the level below it and nothing further, so this key belongs to \
                     whatever claimed that level",
                ));
            }
        };

        let value = backend
            .decode::<V>(&raw)
            .attach_prefix(path)
            .attach_entry(key.as_ref())
            .map_err(|why| WriteValue::from_backend(&full_path, StorageError::Codec, why))?;

        results.push((key, value));
    }

    Ok(results)
}

pub async fn map_update_async<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    key: K,
    value: &V,
    source: Option<Uuid>,
) -> ReactiveMapResult<()>
where
    B: AmeBackend,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let Some(old_value) = core.cache.get(key.as_ref()) else {
        return Err(ReactiveMapError::Absent {
            at: path.entry(key.as_ref()),
        });
    };

    let change = MapChange::Update {
        key,
        old_value: Some(old_value),
        new_value: value.clone(),
        source,
    };

    map_apply_change_async(backend, core, path, change).await
}

pub async fn map_insert_async<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    key: K,
    value: &V,
    source: Option<Uuid>,
) -> ReactiveMapResult<()>
where
    B: AmeBackend,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let change = if let Some(old_value) = core.cache.get(key.as_ref()) {
        MapChange::Update {
            key,
            old_value: Some(old_value),
            new_value: value.clone(),
            source,
        }
    } else {
        MapChange::Insert {
            key,
            value: value.clone(),
            source,
        }
    };

    map_apply_change_async(backend, core, path, change).await
}

pub async fn map_remove_async<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    key: K,
    source: Option<Uuid>,
) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackend,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let Some(old_value) = core.cache.get(key.as_ref()) else {
        return Ok(None);
    };

    let change = MapChange::Remove {
        key,
        old_value: Some(old_value.clone()),
        source,
    };
    map_apply_change_async(backend, core, path, change).await?;
    Ok(Some(old_value))
}

pub async fn map_clear_async<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    source: Option<Uuid>,
) -> ReactiveMapResult<()>
where
    B: AmeBackend,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    map_apply_change_async(backend, core, path, MapChange::Clear { source }).await
}

pub async fn map_apply_change_async<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    change: MapChange<K, V>,
) -> ReactiveMapResult<()>
where
    B: AmeBackend,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let subject = change.key().map(|key| path.entry(key.as_ref()));
    let context_path = subject.clone().unwrap_or_else(|| path.clone());

    let processed = core
        .run_interceptors(context_path.clone(), change)
        .map_err(|refusal| ReactiveMapError::refused(&context_path, refusal))?;

    let source = processed.source();
    let before = held_before(core, &processed);

    map_apply_remote_change(core, &processed);
    core.notify(&processed);

    let written = match &processed {
        MapChange::Insert { key, value, .. }
        | MapChange::Update {
            key,
            new_value: value,
            ..
        } => {
            let entry = path.entry(key.as_ref());
            backend
                .set_with_source(&entry, value, source)
                .await
                .attach_key(&entry)
                .map_err(|why| WriteValue::from_backend(&entry, StorageError::Write, why))
        }
        MapChange::Remove { key, .. } => {
            let entry = path.entry(key.as_ref());
            backend
                .delete_with_source(&entry, source)
                .await
                .attach_key(&entry)
                .map_err(|why| WriteValue::from_backend(&entry, StorageError::Delete, why))
        }
        MapChange::Clear { .. } => backend
            .delete_prefix(&path, source)
            .await
            .attach_prefix(&path)
            .map_err(|why| WriteValue::from_backend(&path, StorageError::Delete, why)),
    };

    if let Err(why) = written {
        for undo in undoing(&processed, before) {
            map_apply_remote_change(core, &undo);
            core.notify(&undo);
        }
        return Err(why.into());
    }

    Ok(())
}

enum HeldBefore<K, V> {
    Entry(K, Option<V>),
    Every(Vec<(K, V)>),
}

fn held_before<K, V>(core: &ReactiveMapCore<K, V>, change: &MapChange<K, V>) -> HeldBefore<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    match change.key() {
        Some(key) => HeldBefore::Entry(key.clone(), core.cache.get(key.as_ref())),
        None => HeldBefore::Every(core.cache.entries().collect()),
    }
}

fn undoing<K, V>(change: &MapChange<K, V>, before: HeldBefore<K, V>) -> Vec<MapChange<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let source = change.source();

    match before {
        HeldBefore::Every(entries) => entries
            .into_iter()
            .map(|(key, value)| MapChange::Insert { key, value, source })
            .collect(),
        HeldBefore::Entry(key, was) => {
            let now = match change {
                MapChange::Insert { value, .. }
                | MapChange::Update {
                    new_value: value, ..
                } => Some(value.clone()),
                MapChange::Remove { .. } | MapChange::Clear { .. } => None,
            };

            match (was, now) {
                (Some(old), Some(new)) => vec![MapChange::Update {
                    key,
                    old_value: Some(new),
                    new_value: old,
                    source,
                }],
                (None, Some(new)) => vec![MapChange::Remove {
                    key,
                    old_value: Some(new),
                    source,
                }],
                (Some(old), None) => vec![MapChange::Insert {
                    key,
                    value: old,
                    source,
                }],
                (None, None) => Vec::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use error_stack::Report;
    use serde::Serialize;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct Refused;

    impl std::fmt::Display for Refused {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("refused")
        }
    }

    impl std::error::Error for Refused {}

    #[derive(Clone, Default)]
    struct Recording {
        stored: Arc<Mutex<BTreeMap<StorePath, serde_json::Value>>>,
        said: Arc<Mutex<Vec<String>>>,
        refuse: bool,
    }

    impl Recording {
        fn wrote(&self, what: String) -> Result<(), Report<Refused>> {
            self.said.lock().unwrap().push(what);
            match self.refuse {
                true => Err(Report::new(Refused)),
                false => Ok(()),
            }
        }
    }

    impl AmeBackend for Recording {
        type Error = Refused;
        type Raw = serde_json::Value;

        async fn get<T>(&self, path: &StorePath) -> Result<Option<T>, Report<Refused>>
        where
            T: DeserializeOwned,
        {
            self.said.lock().unwrap().push(format!("read {path}"));
            Ok(self
                .stored
                .lock()
                .unwrap()
                .get(path)
                .map(|raw| serde_json::from_value(raw.clone()).unwrap()))
        }

        async fn set<T>(&self, path: &StorePath, value: &T) -> Result<(), Report<Refused>>
        where
            T: Serialize,
        {
            self.set_with_source(path, value, None).await
        }

        async fn set_with_source<T: Serialize>(
            &self,
            path: &StorePath,
            value: &T,
            _source: Option<Uuid>,
        ) -> Result<(), Report<Refused>> {
            self.wrote(format!("wrote {path}"))?;
            self.stored
                .lock()
                .unwrap()
                .insert(path.clone(), serde_json::to_value(value).unwrap());
            Ok(())
        }

        async fn set_owned_with_source<T: Serialize>(
            &self,
            path: StorePath,
            value: &T,
            source: Option<Uuid>,
        ) -> Result<(), Report<Refused>> {
            self.set_with_source(&path, value, source).await
        }

        async fn delete(&self, path: &StorePath) -> Result<(), Report<Refused>> {
            self.delete_with_source(path, None).await
        }

        async fn delete_with_source(
            &self,
            path: &StorePath,
            _source: Option<Uuid>,
        ) -> Result<(), Report<Refused>> {
            self.wrote(format!("deleted {path}"))?;
            self.stored.lock().unwrap().remove(path);
            Ok(())
        }

        async fn delete_prefix(
            &self,
            prefix: &StorePath,
            _source: Option<Uuid>,
        ) -> Result<(), Report<Refused>> {
            self.wrote(format!("cleared {prefix}"))?;
            self.stored
                .lock()
                .unwrap()
                .retain(|path, _| !path.starts_with(prefix));
            Ok(())
        }

        async fn scan_prefix(
            &self,
            prefix: &StorePath,
        ) -> Result<Vec<(StorePath, serde_json::Value)>, Report<Refused>> {
            Ok(self
                .stored
                .lock()
                .unwrap()
                .iter()
                .filter(|(path, _)| path.starts_with(prefix))
                .map(|(path, raw)| (path.clone(), raw.clone()))
                .collect())
        }

        async fn scan_keys(&self, prefix: &StorePath) -> Result<Vec<StorePath>, Report<Refused>> {
            Ok(self
                .scan_prefix(prefix)
                .await?
                .into_iter()
                .map(|(path, _)| path)
                .collect())
        }

        fn decode<T>(&self, raw: &serde_json::Value) -> Result<T, Report<Refused>>
        where
            T: DeserializeOwned + Default,
        {
            Ok(serde_json::from_value(raw.clone()).unwrap_or_default())
        }
    }

    fn widths() -> StorePath {
        StorePath::segment("widths")
    }

    fn listening(backend: &Recording) -> (ReactiveMapCore<String, u64>, crate::SignalSubscription) {
        let core = ReactiveMapCore::new();
        let said = backend.said.clone();
        let sub = core.subscribe_any(move |change| {
            let heard = match change {
                MapChange::Insert { key, value, .. } => format!("heard {key} = {value}"),
                MapChange::Update { key, new_value, .. } => format!("heard {key} = {new_value}"),
                MapChange::Remove { key, .. } => format!("heard {key} gone"),
                MapChange::Clear { .. } => "heard cleared".to_string(),
            };
            said.lock().unwrap().push(heard);
        });
        (core, sub)
    }

    #[test]
    fn an_insert_is_heard_before_the_store_answers() {
        let backend = Recording::default();
        let (core, _sub) = listening(&backend);

        futures::executor::block_on(map_insert_async(
            &backend,
            &core,
            widths(),
            "cpu".to_string(),
            &120,
            None,
        ))
        .unwrap();

        assert_eq!(
            *backend.said.lock().unwrap(),
            ["heard cpu = 120", "wrote widths.cpu"]
        );
        assert_eq!(core.cache.get("cpu"), Some(120));
    }

    #[test]
    fn a_refused_insert_is_taken_back() {
        let backend = Recording {
            refuse: true,
            ..Recording::default()
        };
        let (core, _sub) = listening(&backend);

        let refused = futures::executor::block_on(map_insert_async(
            &backend,
            &core,
            widths(),
            "cpu".to_string(),
            &120,
            None,
        ));

        assert!(refused.is_err());
        assert_eq!(
            *backend.said.lock().unwrap(),
            ["heard cpu = 120", "wrote widths.cpu", "heard cpu gone"]
        );
        assert_eq!(core.cache.get("cpu"), None);
    }

    #[test]
    fn a_refused_update_puts_the_old_value_back() {
        let backend = Recording {
            refuse: true,
            ..Recording::default()
        };
        backend
            .stored
            .lock()
            .unwrap()
            .insert(widths().entry("cpu"), serde_json::json!(80));
        let (core, _sub) = listening(&backend);
        core.cache.insert("cpu".to_string(), 80);

        let refused = futures::executor::block_on(map_update_async(
            &backend,
            &core,
            widths(),
            "cpu".to_string(),
            &120,
            None,
        ));

        assert!(refused.is_err());
        assert_eq!(
            *backend.said.lock().unwrap(),
            ["heard cpu = 120", "wrote widths.cpu", "heard cpu = 80"]
        );
        assert_eq!(core.cache.get("cpu"), Some(80));
    }
}
