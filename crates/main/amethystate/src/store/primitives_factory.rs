use crate::observability::Reason;
use crate::observability::register_field;
use crate::reactive::field::Unreadable;
use crate::store::StorageError;
use crate::store::StorageResult;
use crate::store::StoreSubscription;
use crate::store::facts::{Facts, Key, Prefix, Refused};
use crate::store::opening::OpenStruct;
use crate::store::reading::{LoadMap, LoadMapResult};
use crate::store::rules::{OnDelete, OnUnreadable, ReadRules, UnreadableEntries};
use crate::store::traits::{StoreExt as _, StoredAs};
use crate::{Field, ReactiveMap, StateScope, Store, StoreBackend, StoreOp, SubscriptionKind};
use crate::{ReactiveMapKey, ReactiveMapValue};
use amethystate_core::path::{IntoStorePath, Level, PathRef, StorePath};
use amethystate_core::{FieldCore, MapChange, ReactiveMapCore, Signal};
use error_stack::{Report, ResultExt};
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// A field under `TScope`'s path, at the levels `key` names.
pub fn field<TScope, TValue>(
    store: &Store,
    key: impl IntoStorePath,
    default: TValue,
    instance_id: Uuid,
) -> Result<Field<TValue>, OpenStruct>
where
    TScope: StateScope,
    TValue: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    let path = TScope::PATH.join(&key.into_store_path()?);
    field_with_path(store, path, default, instance_id)
}

/// Records that whoever is being built owns this path, or refuses because
/// somebody else already does.
///
/// The claim is the schema's own type name, which is what makes it idempotent:
/// building the same struct twice claims the same path twice and changes
/// nothing. An instance nobody registered claims nothing - there is no name to
/// attribute it to, and refusing what cannot be attributed would be guessing.
fn claim(
    store: &Store,
    path: &StorePath,
    instance_id: Uuid,
) -> Result<(), Box<crate::store::places::Taken>> {
    let Some(by) = crate::store::instances::resolve_instance(instance_id) else {
        return Ok(());
    };

    store.places().take(path, by)
}

pub fn field_with_path<TValue>(
    store: &Store,
    path: impl IntoStorePath,
    default: TValue,
    instance_id: Uuid,
) -> Result<Field<TValue>, OpenStruct>
where
    TValue: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    field_with_path_under(store, path, default, instance_id, ReadRules::new())
}

/// [`field_with_path`] with a say in what a value it cannot read, and a key
/// removed under it, each do.
pub fn field_with_path_where<TValue>(
    store: &Store,
    path: impl IntoStorePath,
    default: TValue,
    instance_id: Uuid,
    policy: OnUnreadable,
    on_delete: OnDelete,
) -> Result<Field<TValue>, OpenStruct>
where
    TValue: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    field_with_path_under(
        store,
        path,
        default,
        instance_id,
        ReadRules::new().on_unreadable(policy).on_delete(on_delete),
    )
}

/// [`field_with_path`] under everything the field declared about disagreeing
/// with the store.
pub fn field_with_path_under<TValue>(
    store: &Store,
    path: impl IntoStorePath,
    default: TValue,
    instance_id: Uuid,
    rules: ReadRules<TValue>,
) -> Result<Field<TValue>, OpenStruct>
where
    TValue: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    let path = path.into_store_path()?;
    let ReadRules {
        on_unreadable: policy,
        on_delete,
        check,
        stored_as,
    } = rules;

    claim(store, &path, instance_id)?;
    register_field::<TValue>(&path, instance_id);

    let mut refused: Option<Reason> = None;

    let current = match read_stored(store, &path, stored_as) {
        Ok(Some(stored)) => match check.map(|check| check(&stored, store.context())) {
            None | Some(Ok(())) => stored,
            Some(Err(invalid)) => {
                if policy == OnUnreadable::Refuse {
                    return Err(OpenStruct::Refused {
                        at: path.clone(),
                        said: Arc::from(invalid.reason()),
                    });
                }

                tracing::error!(
                    path = %path,
                    reason = %invalid,
                    "a declared check refused the stored value, so the field starts on its default"
                );
                refused = Some(Reason::Refused(Arc::from(invalid.reason())));
                default.clone()
            }
        },
        Ok(None) => {
            if let Some(in_the_way) = seed(store, &path, &default, stored_as)? {
                refused = Some(Reason::Occupied(in_the_way));
            }
            default.clone()
        }
        Err(why) if policy.covers(&why) => {
            tracing::error!(path = %path, error = %why, "decode failed while building");
            refused = Some(Reason::WillNotRead(Arc::from(why.to_string().as_str())));
            default.clone()
        }
        Err(why) => {
            return Err(match crate::store::rules::will_not_read(&why) {
                true => OpenStruct::WillNotRead {
                    at: path.clone(),
                    why,
                },
                false => OpenStruct::Store(why),
            });
        }
    };

    let signal = Signal::new(current);

    let sig_clone = signal.clone();
    let store_clone = store.clone();
    let path_log = path.clone();
    let deleted = default.clone();

    let unreadable = Unreadable::new(std::sync::Mutex::new(refused));
    let unreadable_sub = unreadable.clone();

    let id = store.subscribe(
        SubscriptionKind::ExactPath(path.clone()),
        Arc::new(move |event| match &event.new {
            Some(raw) => match match stored_as.read {
                Some(read) => store_clone.decode_with(&event.path, raw, read),
                None => store_clone.decode::<TValue>(raw),
            } {
                Ok(parsed) => {
                    if let Some(check) = check.filter(|_| event.is_external_edit())
                        && let Err(invalid) = check(&parsed, store_clone.context())
                    {
                        if let Ok(mut held) = unreadable_sub.lock() {
                            *held = Some(Reason::Refused(Arc::from(invalid.reason())));
                        }

                        return Err(Report::new(StorageError::Notify)
                            .attach(Key(path_log.clone()))
                            .attach(Refused(invalid.reason().to_string()))
                            .attach("the field kept what it had"));
                    }

                    if let Ok(mut held) = unreadable_sub.lock() {
                        *held = None;
                    }
                    sig_clone.set_forwarded(parsed, event.source.handle());
                    Ok(())
                }
                Err(e) => {
                    tracing::error!(
                        path = %path_log,
                        error = ?e,
                        "a change arrived that will not decode, so the field kept what it had"
                    );

                    if let Ok(mut held) = unreadable_sub.lock() {
                        *held = Some(Reason::WillNotRead(Arc::from(e.to_string().as_str())));
                    }

                    Ok(())
                }
            },
            None => {
                match on_delete {
                    OnDelete::UseDefault => {
                        // Back on the declared default, which is a value
                        // nothing on disk disagrees with: the reason has to go
                        // with the value it was about, or the field reports a
                        // disagreement with a key that is no longer there.
                        if let Ok(mut held) = unreadable_sub.lock() {
                            *held = None;
                        }
                        sig_clone.set_forwarded(deleted.clone(), event.source.handle())
                    }
                    OnDelete::Keep => {}
                }
                Ok(())
            }
        }),
    );

    Ok(Field {
        inner: Arc::new(crate::reactive::field::FieldInner {
            unreadable,
            core: FieldCore::new_with_signal(signal),
            path,
            instance_id,
            store_sub: Some(Arc::new(StoreSubscription::new(store.clone(), id))),
            stored_as,
        }),
    })
}

/// A map under `TScope`'s path, at the levels `key` names.
pub fn reactive_map<TScope, K, V>(
    store: &Store,
    key: impl IntoStorePath,
    default: HashMap<K, V>,
    instance_id: Uuid,
) -> LoadMapResult<ReactiveMap<K, V>>
where
    TScope: StateScope,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let path = TScope::PATH.join(&key.into_store_path()?);
    reactive_map_with_path::<TScope, _, _>(store, path, default, instance_id)
}

pub fn reactive_map_with_path<TScope, K, V>(
    store: &Store,
    path: impl IntoStorePath,
    defaults: HashMap<K, V>,
    instance_id: Uuid,
) -> LoadMapResult<ReactiveMap<K, V>>
where
    TScope: StateScope,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    reactive_map_with_path_only(store, path, defaults, instance_id)
}

/// The value at `path`, read the way the field says rather than the way its
/// type would.
fn read_stored<TValue>(
    store: &Store,
    path: &StorePath,
    stored_as: StoredAs<TValue>,
) -> StorageResult<Option<TValue>>
where
    TValue: DeserializeOwned + 'static,
{
    let Some(read) = stored_as.read else {
        return Ok(store.get::<TValue>(path)?);
    };

    match store.get_raw(path)? {
        Some(bytes) => store.decode_with(path, &bytes, read).map(Some),
        None => Ok(None),
    }
}

/// Writes the field's declared default, and says so if it could not.
///
/// `Some` is what stood in the way. Building carries on - the field takes the
/// default it was declared with - but it is now reporting something the store
/// does not hold, so what came back here goes to [`Reason::Occupied`] and out
/// through [`Field::try_get`](crate::Field::try_get).
fn seed<TValue>(
    store: &Store,
    path: &StorePath,
    default: &TValue,
    stored_as: StoredAs<TValue>,
) -> StorageResult<Option<Arc<str>>>
where
    TValue: Serialize + 'static,
{
    let written = match stored_as.write {
        Some(write) => write(default, &mut |erased| {
            StoreBackend::set_erased(store, path, erased, None)
        }),
        None => store.set(path, default).map_err(Report::from),
    };

    match written {
        Err(report) if report.contains::<crate::store::Occupied>() => {
            Ok(Some(Arc::from(crate::store::one_line(&report).as_str())))
        }
        Err(other) => Err(other),
        Ok(()) => Ok(None),
    }
}

/// Every entry stored under `path`, keyed by the level below it.
///
/// A key that cannot be read back is an error rather than an absence. The path
/// itself is not an entry.
pub fn load_map<K, V>(store: &Store, path: &StorePath) -> LoadMapResult<IndexMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    load_map_where(store, path, UnreadableEntries::Refuse)
}

/// [`load_map`] with a say in what an entry it cannot read does.
///
/// Under [`UnreadableEntries::Skip`] such an entry is left on disk, left out of
/// the map, and named in a line at `error`; everything else the scan can
/// disagree with still refuses.
pub fn load_map_where<K, V>(
    store: &Store,
    path: &StorePath,
    policy: UnreadableEntries,
) -> LoadMapResult<IndexMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    if store.parallel_reads() {
        use rayon::prelude::*;

        let scanned = StoreBackend::scan_prefix(store, path)
            .attach_prefix(path)
            .map_err(|why| LoadMap::from_store(path, why))?;

        if scanned.len() >= PARALLEL_MIN_LEN {
            let decoded = scanned
                .par_iter()
                .with_min_len(PARALLEL_MIN_LEN)
                .filter_map(|(stored, bytes)| {
                    decode_entry(store, path, PathRef::from(stored), bytes, policy).transpose()
                })
                .collect::<LoadMapResult<Vec<(K, V)>>>();

            return match decoded {
                Ok(entries) => Ok(entries.into_iter().collect()),
                Err(whichever) => {
                    Err(first_undecodable::<K, V>(store, path, &scanned, policy)
                        .unwrap_or(whichever))
                }
            };
        }

        let mut entries = IndexMap::with_capacity(scanned.len());
        for (stored, bytes) in &scanned {
            if let Some((key, value)) =
                decode_entry(store, path, PathRef::from(stored), bytes, policy)?
            {
                entries.insert(key, value);
            }
        }
        return Ok(entries);
    }

    let mut entries = IndexMap::new();
    let mut refused: Option<LoadMap> = None;

    let visited = store.visit_prefix(path, &mut |key, bytes| match decode_entry(
        store, path, key, bytes, policy,
    ) {
        Ok(Some((k, v))) => {
            entries.insert(k, v);
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(why) => {
            let stop = Report::new(StorageError::Read).attach("an entry the map would not take");
            refused = Some(why);
            Err(stop)
        }
    });

    match (refused, visited) {
        (Some(why), _) => Err(why),
        (None, Err(why)) => Err(LoadMap::from_store(path, why)),
        (None, Ok(())) => Ok(entries),
    }
}

const PARALLEL_MIN_LEN: usize = 1024;

/// The first entry that will not decode, in the order the store handed them
/// over.
///
/// Rayon keeps whichever refusal a worker recorded first, which is a race
/// between threads rather than a fact about the data. A read that came back
/// with one asks this instead, so a divided read blames the entry the
/// undivided one stops at - and a caller learns the same thing about their map
/// either way. Only a read that already failed pays for it.
fn first_undecodable<K, V>(
    store: &Store,
    path: &StorePath,
    scanned: &[(StorePath, Vec<u8>)],
    policy: UnreadableEntries,
) -> Option<LoadMap>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    scanned.iter().find_map(|(stored, bytes)| {
        decode_entry::<K, V>(store, path, PathRef::from(stored), bytes, policy).err()
    })
}

fn decode_entry<K, V>(
    store: &Store,
    path: &StorePath,
    stored: PathRef<'_>,
    bytes: &[u8],
    policy: UnreadableEntries,
) -> LoadMapResult<Option<(K, V)>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    match read_entry::<K, V>(store, path, stored, bytes) {
        Err(why) if left_out(&why, policy) => {
            tracing::error!(
                entry = %stored.as_str(),
                reason = %why,
                "the map was built to carry on without what it cannot read, so this entry \
                 stays on disk and out of the map"
            );
            Ok(None)
        }
        other => other,
    }
}

/// Whether [`UnreadableEntries::Skip`] answers this by leaving the entry out.
///
/// A codec refusal and nothing else: a key that is not an entry of this map is
/// a question about places, and is refused under either answer.
fn left_out(why: &LoadMap, policy: UnreadableEntries) -> bool {
    if policy == UnreadableEntries::Refuse {
        return false;
    }

    match why {
        LoadMap::KeyWillNotRead { .. } => true,
        LoadMap::EntryWillNotRead { why, .. } => crate::store::rules::will_not_read(why),
        _ => false,
    }
}

fn read_entry<K, V>(
    store: &Store,
    path: &StorePath,
    stored: PathRef<'_>,
    bytes: &[u8],
) -> LoadMapResult<Option<(K, V)>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let below = stored.level_under(path);

    let name = match &below {
        Level::Entry(name) => name.as_ref(),
        Level::Prefix => return Ok(None),
        Level::Deeper(_) => {
            return Err(LoadMap::KeyIsNotAnEntry {
                under: path.clone(),
                stored: Arc::from(stored.as_str()),
                said: Arc::from(
                    "a map owns the level below it and nothing further, so this key \
                     belongs to whatever claimed that level",
                ),
            });
        }
        Level::Outside => {
            return Err(LoadMap::KeyIsNotAnEntry {
                under: path.clone(),
                stored: Arc::from(stored.as_str()),
                said: Arc::from("the key is not under the map it was scanned from"),
            });
        }
    };

    let key = K::from_str(name).map_err(|_| LoadMap::KeyWillNotRead {
        under: path.clone(),
        entry: Arc::from(name),
        wanted: std::any::type_name::<K>(),
    })?;

    let entry = path.join(&StorePath::segment(name));
    let value = store
        .decode::<V>(bytes)
        .map_err(Report::from)
        .attach_prefix(path)
        .attach_entry(name)
        .map_err(|why| LoadMap::from_store(&entry, why))?;

    Ok(Some((key, value)))
}

pub fn reactive_map_with_path_only<K, V>(
    store: &Store,
    path: impl IntoStorePath,
    defaults: HashMap<K, V>,
    instance_id: Uuid,
) -> LoadMapResult<ReactiveMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    reactive_map_where(
        store,
        path,
        defaults,
        instance_id,
        UnreadableEntries::default(),
        OnDelete::default(),
    )
}

/// [`reactive_map_with_path_only`] with a say in what an entry it cannot read,
/// and the loss of the level it sits at, each do.
///
/// [`UnreadableEntries`] is the map's own answer about its entries, spelled
/// apart from [`OnUnreadable`] because standing a default in for one entry is
/// not a thing a map can do.
///
/// [`OnDelete`] is about the level, since the entries under it are data: an
/// entry somebody removed is a removal under either answer, or the map would go
/// on reporting a key the store no longer holds.
/// [`OnDelete::UseDefault`] puts the declared entries back in the map when the
/// level goes, the way a field goes back to its default; [`OnDelete::Keep`]
/// leaves the map as the store left it, which is empty.
pub fn reactive_map_where<K, V>(
    store: &Store,
    path: impl IntoStorePath,
    defaults: HashMap<K, V>,
    instance_id: Uuid,
    policy: UnreadableEntries,
    on_delete: OnDelete,
) -> LoadMapResult<ReactiveMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let path = path.into_store_path()?;
    claim(store, &path, instance_id)?;

    let declared = match on_delete {
        OnDelete::UseDefault => defaults.clone(),
        OnDelete::Keep => HashMap::new(),
    };

    let mut known_cache = load_map_where::<K, V>(store, &path, policy)?;

    let seeded_before = store.is_initialized(&path)? || !known_cache.is_empty();

    if !seeded_before {
        for (k, v) in defaults {
            let full_path = path
                .try_push(k.to_string())
                .change_context(StorageError::Path)
                .attach_prefix(&path)
                .attach_entry(&k.to_string())?;
            store.set(&full_path, &v)?;
            known_cache.insert(k, v);
        }
    }
    store.mark_initialized(&path)?;

    let core = ReactiveMapCore::with_capacity(known_cache.len());
    for (k, v) in known_cache {
        core.cache.insert(k, v);
    }

    let core_clone = core.clone();
    let map_path = path.clone();
    let path_for_keys = path.clone();
    let store_clone = store.clone();
    let id = store.subscribe(
        SubscriptionKind::Prefix(path.clone()),
        Arc::new(move |event| {
            if event.op == StoreOp::DeletePrefix && event.path == map_path {
                core_clone.cache.clear();
                core_clone.notify(&MapChange::Clear {
                    source: event.source.handle(),
                });

                // Only under `OnDelete::UseDefault`, where `declared` is what
                // the map was built with; otherwise it is empty and this is a
                // walk over nothing. In the cache rather than on disk, the way
                // a field takes its default without writing it back: the store
                // was told to forget, and putting it all back is not this
                // map's decision to make.
                for (key, value) in &declared {
                    core_clone.cache.insert(key.clone(), value.clone());
                    core_clone.notify(&MapChange::Insert {
                        key: key.clone(),
                        value: value.clone(),
                        source: event.source.handle(),
                    });
                }
                return Ok(());
            }

            let Some(key_str) = path_for_keys.entry_name(&event.path) else {
                return Err(Report::new(StorageError::Notify)
                    .attach(Key(event.path.clone()))
                    .attach(Prefix(path_for_keys.clone()))
                    .attach("not a path this library could have written, so the map did not take it"));
            };

            let Ok(k) = K::from_str(&key_str) else {
                return Err(Report::new(StorageError::Notify)
                    .attach(Key(event.path.clone()))
                    .attach(Prefix(path_for_keys.clone()))
                    .attach(format!("does not parse as {}", std::any::type_name::<K>())));
            };

            {
                let source = event.source.handle();

                let new_val = match event.new.as_ref().map(|b| store_clone.decode::<V>(b)) {
                    Some(Ok(value)) => Some(value),
                    Some(Err(e)) => {
                        return Err(e
                            .change_context(StorageError::Notify)
                            .attach(Key(event.path.clone()))
                            .attach("the map kept what it had"));
                    }
                    None => None,
                };

                let stored_old = match event.old.as_ref().map(|b| store_clone.decode::<V>(b)) {
                    Some(Ok(value)) => Some(value),
                    Some(Err(e)) => {
                        tracing::warn!(
                            path = %event.path,
                            "the value being replaced would not read as this map's value type, so what this map last held is what subscribers are told: {e:?}"
                        );
                        None
                    }
                    None => None,
                };

                let old_val = stored_old.or_else(|| core_clone.cache.get(&k));

                let change = {
                    let keys = &core_clone.cache;

                    match event.op {
                        StoreOp::Set => {
                            let Some(new_value) = new_val else {
                                return Err(Report::new(StorageError::Notify)
                                    .attach(Key(event.path.clone()))
                                    .attach("a set carried no value, so the map kept what it had"));
                            };

                            if keys.contains_key(&k) {
                                let old_value = old_val;
                                keys.insert(k.clone(), new_value.clone());
                                MapChange::Update {
                                    key: k.clone(),
                                    old_value,
                                    new_value,
                                    source,
                                }
                            } else {
                                keys.insert(k.clone(), new_value.clone());
                                MapChange::Insert {
                                    key: k.clone(),
                                    value: new_value,
                                    source,
                                }
                            }
                        }
                        StoreOp::Delete | StoreOp::DeletePrefix => {
                            keys.remove(&k);
                            MapChange::Remove {
                                key: k.clone(),
                                old_value: old_val,
                                source,
                            }
                        }
                    }
                };

                core_clone.notify(&change);
            }

            Ok(())
        }),
    );

    Ok(ReactiveMap {
        inner: Arc::new(crate::reactive::map::MapInner {
            core,
            path,
            instance_id,
            store: store.clone(),
            store_sub: Arc::new(StoreSubscription::new(store.clone(), id)),
        }),
    })
}
