use crate::AmeBackendSync;
use crate::path::StorePath;
use crate::primitives::error::WriteError;
use crate::primitives::error::{ReactiveMapError, ReactiveMapResult};
use crate::primitives::map_core::{MapEntryPath, ReactiveMapKey, ReactiveMapValue};
use crate::{MapChange, ReactiveMapCore};
use crate::facts::{Facts, Prefix};
use error_stack::{Report, ResultExt};
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// Writes a key that already exists, and fails with
/// [`ReactiveMapError::KeyNotFound`] otherwise.
pub fn map_update<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    key: K,
    value: &V,
    source: Option<Uuid>,
) -> ReactiveMapResult<()>
where
    B: AmeBackendSync,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let full_path = path.entry(&key)?;
    let old_value = match read_entry::<B, V>(backend, &full_path)? {
        Some(old_value) => old_value,
        None => {
            return Err(
                Report::new(ReactiveMapError::KeyNotFound(key.to_string()))
                    .attach(Prefix(path.clone())),
            );
        }
    };

    let change = MapChange::Update {
        key,
        old_value,
        new_value: value.clone(),
        source,
    };

    map_apply_change(backend, core, path, change)
}

/// Writes a key whether or not it exists, emitting [`MapChange::Insert`] for
/// a new one and [`MapChange::Update`] for one that was already there.
pub fn map_insert<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    key: K,
    value: &V,
    source: Option<Uuid>,
) -> ReactiveMapResult<()>
where
    B: AmeBackendSync,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let full_path = path.entry(&key)?;
    let old_value = read_entry::<B, V>(backend, &full_path)?;
    let change = if let Some(old_value) = old_value {
        MapChange::Update {
            key,
            old_value,
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

    map_apply_change(backend, core, path, change)
}

pub fn map_remove<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    key: K,
    source: Option<Uuid>,
) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackendSync,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let exists = core.cache.contains_key(&key);
    if !exists {
        return Ok(None);
    }

    let full_path = path.entry(&key)?;
    let old_value = read_entry::<B, V>(backend, &full_path)?;
    if let Some(old_value) = old_value {
        let change = MapChange::Remove {
            key,
            old_value: old_value.clone(),
            source,
        };
        map_apply_change(backend, core, path, change)?;
        Ok(Some(old_value))
    } else {
        core.cache.remove(&key);
        Ok(None)
    }
}

fn read_entry<B, V>(backend: &B, entry: &StorePath) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackendSync,
    V: DeserializeOwned,
{
    backend
        .get::<V>(entry)
        .change_context(WriteError::Storage)
        .attach_key(entry)
}

pub fn map_clear<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    source: Option<Uuid>,
) -> ReactiveMapResult<()>
where
    B: AmeBackendSync,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    map_apply_change(backend, core, path, MapChange::Clear { source })
}

pub fn map_apply_change<B, K, V>(
    backend: &B,
    core: &ReactiveMapCore<K, V>,
    path: StorePath,
    change: MapChange<K, V>,
) -> ReactiveMapResult<()>
where
    B: AmeBackendSync,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let subject = match change.key() {
        Some(key) => Some(path.entry(key)?),
        None => None,
    };
    let context_path = subject.clone().unwrap_or_else(|| path.clone());

    let processed = core
        .run_interceptors(context_path, change)
        .map_err(ReactiveMapError::intercepted)
        .attach_prefix(&path)
        .attach_with(|| match &subject {
            Some(entry) => format!("affects: {entry}"),
            None => format!("affects: all of {path}"),
        })?;

    match &processed {
        MapChange::Insert { key, value, .. }
        | MapChange::Update {
            key,
            new_value: value,
            ..
        } => {
            let entry = path.entry(key)?;
            backend
                .set_with_source(&entry, value, processed.source())
                .change_context(WriteError::Storage)
                .attach_key(&entry)?;
        }
        MapChange::Remove { key, .. } => {
            let entry = path.entry(key)?;
            backend
                .delete_with_source(&entry, processed.source())
                .change_context(WriteError::Storage)
                .attach_key(&entry)?;
        }
        MapChange::Clear { .. } => {
            backend
                .delete_prefix(&path, processed.source())
                .change_context(WriteError::Storage)
                .attach_prefix(&path)?;
        }
    }

    map_apply_remote_change(core, &processed);

    Ok(())
}

/// The only writer to the key cache, and it has to stay that way.
///
/// It runs off the store subscription, so writes made here and edits made to
/// the file from outside arrive down the same path - which is what keeps the
/// cache and the store agreeing about which keys exist.
///
/// Add a second writer and they drift, and the failure is silent rather than
/// loud: `remove` gates on the cache, so a key the cache has lost answers
/// `Ok(None)` and deletes nothing, on a key that is really in the store.
pub fn map_apply_remote_change<K, V>(core: &ReactiveMapCore<K, V>, change: &MapChange<K, V>)
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let keys = &core.cache;
    match change {
        MapChange::Insert { key, value, .. }
        | MapChange::Update {
            key,
            new_value: value,
            ..
        } => {
            keys.insert(key.clone(), value.clone());
        }
        MapChange::Remove { key, .. } => {
            keys.remove(key);
        }
        MapChange::Clear { .. } => {
            keys.clear();
        }
    }
}
