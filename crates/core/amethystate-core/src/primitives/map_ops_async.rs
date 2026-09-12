use crate::AmeBackendAsync as AmeBackend;
use crate::facts::Facts;
use crate::failure::StorageError;
use crate::path::StorePath;
use crate::primitives::error::{ReactiveMapError, ReactiveMapResult, WriteValue};
use crate::primitives::map_core::{MapEntryPath, ReactiveMapKey, ReactiveMapValue};
use crate::{MapChange, ReactiveMapCore, map_apply_remote_change};
use uuid::Uuid;

use serde::de::DeserializeOwned;

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
        let Some(key_str) = path.entry_name(&full_path) else {
            continue;
        };
        let Some(key) = K::read(key_str.as_str()) else {
            continue;
        };

        let value = backend
            .decode::<V>(&raw)
            .attach_prefix(path)
            .attach_entry(key_str.as_str())
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
    let full_path = path.entry(key.as_ref());
    let old_value = match read_entry::<B, V>(backend, &full_path).await? {
        Some(old_value) => old_value,
        None => return Err(ReactiveMapError::Absent { at: full_path }),
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
    let full_path = path.entry(key.as_ref());
    let old_value = read_entry::<B, V>(backend, &full_path).await?;
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
    let exists = core.cache.contains_key(key.as_ref());
    if !exists {
        return Ok(None);
    }

    let full_path = path.entry(key.as_ref());
    let old_value = read_entry::<B, V>(backend, &full_path).await?;
    if let Some(old_value) = old_value {
        let change = MapChange::Remove {
            key,
            old_value: Some(old_value.clone()),
            source,
        };
        map_apply_change_async(backend, core, path, change).await?;
        Ok(Some(old_value))
    } else {
        core.cache.remove(key.as_ref());
        Ok(None)
    }
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
        .map_err(|said| ReactiveMapError::intercepted(&context_path, said))?;

    let source = processed.source();

    match &processed {
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
                .map_err(|why| WriteValue::from_backend(&entry, StorageError::Write, why))?;
        }
        MapChange::Remove { key, .. } => {
            let entry = path.entry(key.as_ref());
            backend
                .delete_with_source(&entry, source)
                .await
                .attach_key(&entry)
                .map_err(|why| WriteValue::from_backend(&entry, StorageError::Delete, why))?;
        }
        MapChange::Clear { .. } => {
            backend
                .delete_prefix(&path, source)
                .await
                .attach_prefix(&path)
                .map_err(|why| WriteValue::from_backend(&path, StorageError::Delete, why))?;
        }
    }

    map_apply_remote_change(core, &processed);

    Ok(())
}
