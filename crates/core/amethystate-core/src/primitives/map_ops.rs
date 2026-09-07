use crate::AmeBackendSync;
use crate::facts::Facts;
use crate::path::StorePath;
use crate::primitives::error::{ReactiveMapError, ReactiveMapResult, WriteValue};
use crate::primitives::map_core::{MapEntryPath, ReactiveMapKey, ReactiveMapValue};
use crate::{MapChange, ReactiveMapCore};
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// Writes a key that already exists, and fails with
/// [`ReactiveMapError::Absent`] otherwise.
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
        None => return Err(ReactiveMapError::Absent { at: full_path }),
    };

    let change = MapChange::Update {
        key,
        old_value: Some(old_value),
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
            old_value: Some(old_value.clone()),
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
        .attach_key(entry)
        .map_err(|why| WriteValue::from_store(entry, why))
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
        .run_interceptors(context_path.clone(), change)
        .map_err(|said| ReactiveMapError::intercepted(&context_path, said))?;

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
                .attach_key(&entry)
                .map_err(|why| WriteValue::from_store(&entry, why))?;
        }
        MapChange::Remove { key, .. } => {
            let entry = path.entry(key)?;
            backend
                .delete_with_source(&entry, processed.source())
                .attach_key(&entry)
                .map_err(|why| WriteValue::from_store(&entry, why))?;
        }
        MapChange::Clear { .. } => {
            backend
                .delete_prefix(&path, processed.source())
                .attach_prefix(&path)
                .map_err(|why| WriteValue::from_store(&path, why))?;
        }
    }

    map_apply_remote_change(core, &processed);

    Ok(())
}

/// Brings the key cache in line with a change, whether it was made here or
/// read back from an edit to the file.
///
/// The cache and the store have to agree about which keys exist, and when they
/// drift the failure is silent rather than loud: `remove` gates on the cache,
/// so a key the cache has lost answers `Ok(None)` and deletes nothing, on a
/// key that is really in the store.
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
