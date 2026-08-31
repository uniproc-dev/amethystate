use crate::AmeBackendAsync as AmeBackend;
use crate::path::StorePath;
use crate::primitives::error::WriteError;
use crate::primitives::error::{ReactiveMapError, ReactiveMapResult};
use crate::primitives::map_core::{MapEntryPath, ReactiveMapKey, ReactiveMapValue};
use crate::facts::{Facts, Prefix};
use crate::{MapChange, ReactiveMapCore, map_apply_remote_change};
use error_stack::{Report, ResultExt};
use uuid::Uuid;

use serde::de::DeserializeOwned;
use std::fmt::Display;
use std::str::FromStr;

async fn read_entry<B, V>(backend: &B, entry: &StorePath) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackend,
    V: DeserializeOwned,
{
    backend
        .get::<V>(entry)
        .await
        .change_context(WriteError::Storage)
        .attach_key(entry)
}

pub async fn map_get_async<B, K, V>(
    backend: &B,
    path: &StorePath,
    key: &K,
) -> ReactiveMapResult<Option<V>>
where
    B: AmeBackend,
    K: Display,
    V: DeserializeOwned,
{
    let entry = path.entry(key)?;
    read_entry::<B, V>(backend, &entry).await
}

pub async fn map_entries_async<B, K, V>(
    backend: &B,
    path: &StorePath,
) -> ReactiveMapResult<Vec<(K, V)>>
where
    B: AmeBackend,
    K: FromStr,
    V: DeserializeOwned + Default,
{
    let kvs = backend
        .scan_prefix(path)
        .await
        .change_context(WriteError::Storage)
        .attach_prefix(path)?;
    let mut results = Vec::new();

    for (full_path, raw) in kvs {
        let Some(key_str) = full_path
            .strip_prefix(path)
            .as_ref()
            .and_then(StorePath::name)
            .map(|name| name.into_owned())
        else {
            continue;
        };
        let Ok(key) = K::from_str(&key_str) else {
            continue;
        };

        let value = backend
            .decode::<V>(&raw)
            .change_context(WriteError::Storage)
            .attach_prefix(path)
            .attach_entry(&key_str)?;

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
    let full_path = path.entry(&key)?;
    let old_value = match read_entry::<B, V>(backend, &full_path).await? {
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
    let full_path = path.entry(&key)?;
    let old_value = read_entry::<B, V>(backend, &full_path).await?;
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
    let exists = core.cache.contains_key(&key);
    if !exists {
        return Ok(None);
    }

    let full_path = path.entry(&key)?;
    let old_value = read_entry::<B, V>(backend, &full_path).await?;
    if let Some(old_value) = old_value {
        let change = MapChange::Remove {
            key,
            old_value: old_value.clone(),
            source,
        };
        map_apply_change_async(backend, core, path, change).await?;
        Ok(Some(old_value))
    } else {
        core.cache.remove(&key);
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

    let source = processed.source();

    match &processed {
        MapChange::Insert { key, value, .. }
        | MapChange::Update {
            key,
            new_value: value,
            ..
        } => {
            let entry = path.entry(key)?;
            backend
                .set_with_source(&entry, value, source)
                .await
                .change_context(WriteError::Storage)
                .attach_key(&entry)?;
        }
        MapChange::Remove { key, .. } => {
            let entry = path.entry(key)?;
            backend
                .delete_with_source(&entry, source)
                .await
                .change_context(WriteError::Storage)
                .attach_key(&entry)?;
        }
        MapChange::Clear { .. } => {
            backend
                .delete_prefix(&path, source)
                .await
                .change_context(WriteError::Storage)
                .attach_prefix(&path)?;
        }
    }

    map_apply_remote_change(core, &processed);

    Ok(())
}
