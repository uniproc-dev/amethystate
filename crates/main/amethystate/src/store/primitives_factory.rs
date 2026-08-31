use crate::observability::register_field;
use crate::store::StorageError;
use crate::store::StorageResult;
use crate::store::StoreSubscription;
use crate::store::facts::{Entry, Facts, Prefix, RawKey};
use crate::{Field, ReactiveMap, StateScope, Store, StoreBackend, StoreOp, SubscriptionKind};
use crate::{ReactiveMapKey, ReactiveMapValue};
use amethystate_core::path::{IntoStorePath, Level, StorePath};
use amethystate_core::{FieldCore, MapChange, ReactiveMapCore, Signal};
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// What building a struct does about a stored value it will not accept: one
/// that does not decode into the field's type, and one a declared check
/// refuses.
///
/// The value got there somehow - a file edited by hand, a migration that left
/// something behind, a codec that took what it cannot read back - and the two
/// answers serve different applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnUnreadable {
    /// Construction fails, naming the path. Nothing half-built is handed out.
    #[default]
    Refuse,

    /// The field takes its declared default and construction carries on.
    ///
    /// The stored value is left where it is, so a person can still fix the file
    /// by hand, and the field says the store does not agree with what it is
    /// reporting: [`Field::try_get`](crate::Field::try_get) answers `Err` from
    /// the moment it is built until a change decodes.
    UseDefault,
}

impl OnUnreadable {
    /// Whether this failure is one [`OnUnreadable::UseDefault`] stands in for.
    ///
    /// A decode failure, and that alone. A store that cannot be read at all
    /// propagates: there is no default to stand in for a file that is not
    /// there.
    fn covers(&self, why: &Report<StorageError>) -> bool {
        matches!(self, OnUnreadable::UseDefault)
            && why
                .frames()
                .filter_map(|frame| frame.downcast_ref::<StorageError>())
                .any(|context| *context == StorageError::Codec)
    }
}

/// What a field does when its key is deleted under it.
///
/// A deletion is somebody else's doing - another handle, a migration, a hand
/// edited file - and the two answers disagree about what a field is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnDelete {
    /// The field goes on reporting the last value it held.
    ///
    /// A deleted key is not a value, and the declared default is a
    /// compile-time guess - the least likely thing the person was looking at.
    /// Keeping is also what stops a removal and an undecodable value from
    /// being the same observable, which everything else here works to keep
    /// apart.
    #[default]
    Keep,

    /// The field reports its declared default again, as if it had never been
    /// written.
    UseDefault,
}

/// What a field does about the store disagreeing with it: a value it cannot
/// read, a key removed under it, and a value its declared check refuses.
///
/// One value carries all of it, so "what did this field decide" has a single
/// answer to hold and a single place to add to.
pub struct ReadRules<TValue> {
    on_unreadable: OnUnreadable,
    on_delete: OnDelete,
    check: Option<crate::store::Check<TValue>>,
}

impl<TValue> Default for ReadRules<TValue> {
    fn default() -> Self {
        Self {
            on_unreadable: OnUnreadable::default(),
            on_delete: OnDelete::default(),
            check: None,
        }
    }
}

impl<TValue> ReadRules<TValue> {
    /// The rules a field takes when nothing says otherwise: refuse a value
    /// that will not decode, keep what it holds when the key is removed, and
    /// judge nothing.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_unreadable(mut self, policy: OnUnreadable) -> Self {
        self.on_unreadable = policy;
        self
    }

    pub fn on_delete(mut self, policy: OnDelete) -> Self {
        self.on_delete = policy;
        self
    }

    /// The rule every value coming in from the store has to pass.
    pub fn check(mut self, check: crate::store::Check<TValue>) -> Self {
        self.check = Some(check);
        self
    }
}

/// What a declared struct wrote about reading, so the struct holding it can be
/// checked against it while it compiles.
///
/// `None` is a struct that said nothing and takes whatever it is built under.
/// The macro implements this for everything it generates; nothing else should.
pub trait DeclaredPolicy {
    const ON_UNREADABLE: Option<OnUnreadable>;
    const ON_DELETE: Option<OnDelete>;
}

/// A field under `TScope`'s path, at the levels `key` names.
pub fn field<TScope, TValue>(
    store: &Store,
    key: impl IntoStorePath,
    default: TValue,
    instance_id: Uuid,
) -> StorageResult<Field<TValue>>
where
    TScope: StateScope,
    TValue: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
{
    let path = TScope::PATH.join(&crate::store::to_path(key)?);
    field_with_path(store, path, default, instance_id)
}

/// Records that whoever is being built owns this path, or refuses because
/// somebody else already does.
///
/// The claim is the schema's own type name, which is what makes it idempotent:
/// building the same struct twice claims the same path twice and changes
/// nothing. An instance nobody registered claims nothing - there is no name to
/// attribute it to, and refusing what cannot be attributed would be guessing.
fn claim(store: &Store, path: &StorePath, instance_id: Uuid) -> StorageResult<()> {
    match crate::observability::resolve_instance(instance_id) {
        Some(by) => store.owners().claim(path, by),
        None => Ok(()),
    }
}

pub fn field_with_path<TValue>(
    store: &Store,
    path: impl IntoStorePath,
    default: TValue,
    instance_id: Uuid,
) -> StorageResult<Field<TValue>>
where
    TValue: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
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
) -> StorageResult<Field<TValue>>
where
    TValue: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
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
) -> StorageResult<Field<TValue>>
where
    TValue: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
{
    let path = crate::store::to_path(path)?;
    let ReadRules {
        on_unreadable: policy,
        on_delete,
        check,
    } = rules;

    claim(store, &path, instance_id)?;
    register_field::<TValue>(&path, instance_id);

    let mut refused: Option<Arc<str>> = None;

    let current = match store.get::<TValue>(&path) {
        Ok(Some(stored)) => match check.map(|check| check(&stored, store.context())) {
            None | Some(Ok(())) => stored,
            Some(Err(invalid)) => {
                if policy == OnUnreadable::Refuse {
                    return Err(crate::store::refused(&path, &invalid));
                }

                tracing::error!(
                    path = %path,
                    reason = %invalid,
                    "a declared check refused the stored value, so the field starts on its default"
                );
                refused = Some(Arc::from(invalid.reason()));
                default.clone()
            }
        },
        Ok(None) => {
            seed(store, &path, &default)?;
            default.clone()
        }
        Err(why) if policy.covers(&why) => {
            tracing::error!(path = %path, error = %why, "decode failed while building");
            refused = Some(Arc::from(why.to_string().as_str()));
            default.clone()
        }
        Err(why) => return Err(why),
    };

    let signal = Signal::new(current);

    let sig_clone = signal.clone();
    let store_clone = store.clone();
    let path_log = path.clone();
    let deleted = default.clone();

    let unreadable = crate::reactive::field::Unreadable::new(std::sync::Mutex::new(refused));
    let unreadable_sub = unreadable.clone();

    let id = store.subscribe(
        SubscriptionKind::ExactPath(path.clone()),
        Arc::new(move |event| match &event.new {
            Some(raw) => match store_clone.decode::<TValue>(raw) {
                Ok(parsed) => {
                    if let Some(check) = check.filter(|_| event.is_external_edit())
                        && let Err(invalid) = check(&parsed, store_clone.context())
                    {
                        tracing::error!(
                            path = %path_log,
                            reason = %invalid,
                            "a declared check refused an edit from outside, so the field kept what it had"
                        );
                        if let Ok(mut held) = unreadable_sub.lock() {
                            *held = Some(Arc::from(invalid.reason()));
                        }
                        return;
                    }

                    if let Ok(mut held) = unreadable_sub.lock() {
                        *held = None;
                    }
                    sig_clone.set_forwarded(parsed, event.source)
                }
                Err(e) => {
                    tracing::error!(path = %path_log, error = %e, "decode failed");
                    if let Ok(mut held) = unreadable_sub.lock() {
                        *held = Some(Arc::from(e.to_string().as_str()));
                    }
                }
            },
            None => match on_delete {
                OnDelete::UseDefault => sig_clone.set_forwarded(deleted.clone(), event.source),
                OnDelete::Keep => {}
            },
        }),
    );

    Ok(Field {
        inner: Arc::new(crate::reactive::field::FieldInner {
            unreadable,
            core: FieldCore::new_with_signal(signal),
            path,
            instance_id,
            store_sub: Some(Arc::new(StoreSubscription::new(store.clone(), id))),
        }),
    })
}

/// A map under `TScope`'s path, at the levels `key` names.
pub fn reactive_map<TScope, K, V>(
    store: &Store,
    key: impl IntoStorePath,
    default: HashMap<K, V>,
    instance_id: Uuid,
) -> StorageResult<ReactiveMap<K, V>>
where
    TScope: StateScope,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let path = TScope::PATH.join(&crate::store::to_path(key)?);
    reactive_map_with_path::<TScope, _, _>(store, path, default, instance_id)
}

pub fn reactive_map_with_path<TScope, K, V>(
    store: &Store,
    path: impl IntoStorePath,
    defaults: HashMap<K, V>,
    instance_id: Uuid,
) -> StorageResult<ReactiveMap<K, V>>
where
    TScope: StateScope,
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    reactive_map_with_path_only(store, path, defaults, instance_id)
}

fn seed<TValue>(store: &Store, path: &StorePath, default: &TValue) -> StorageResult<()>
where
    TValue: Serialize,
{
    match store.set(path, default) {
        Err(report) if report.contains::<crate::store::Occupied>() => {
            tracing::warn!(
                target: "amethystate",
                path = %path,
                error = %crate::store::one_line(&report),
                "the field starts on its default: the store already holds something in the way, \
                 and seeding over it would destroy it",
            );
            Ok(())
        }
        other => other,
    }
}

/// Every entry stored under `path`, keyed by the level below it.
///
/// A key that cannot be read back is an error rather than an absence. The path
/// itself is not an entry.
pub fn load_map<K, V>(store: &Store, path: &StorePath) -> StorageResult<HashMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    if store.parallel_reads() {
        use rayon::prelude::*;

        let scanned = store
            .scan_prefix(path)
            .attach_prefix(path)?;

        if scanned.len() >= PARALLEL_MIN_LEN {
            let decoded: Vec<(K, V)> = scanned
                .par_iter()
                .with_min_len(PARALLEL_MIN_LEN)
                .filter_map(|(stored, bytes)| {
                    decode_entry(store, path, stored.as_str(), bytes).transpose()
                })
                .collect::<StorageResult<Vec<_>>>()?;

            return Ok(decoded.into_iter().collect());
        }

        let mut entries = HashMap::with_capacity(scanned.len());
        for (stored, bytes) in &scanned {
            if let Some((key, value)) = decode_entry(store, path, stored.as_str(), bytes)? {
                entries.insert(key, value);
            }
        }
        return Ok(entries);
    }

    let mut entries = HashMap::new();
    store.visit_prefix(path, &mut |key, bytes| {
        if let Some((k, v)) = decode_entry(store, path, key, bytes)? {
            entries.insert(k, v);
        }
        Ok(())
    })?;

    Ok(entries)
}

const PARALLEL_MIN_LEN: usize = 1024;

fn decode_entry<K, V>(
    store: &Store,
    path: &StorePath,
    stored: &str,
    bytes: &[u8],
) -> StorageResult<Option<(K, V)>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let below = amethystate_core::path::level_under(stored, path)
        .change_context(StorageError::Path)
        .attach_prefix(path)
        .attach_raw_key(stored)?;

    let name = match &below {
        Level::Entry(name) => name.as_ref(),
        Level::Prefix => return Ok(None),
        Level::Deeper(name) => {
            return Err(Report::new(StorageError::Path)
                .attach(Prefix(path.clone()))
                .attach(RawKey(stored.to_owned()))
                .attach(Entry(name.to_string()))
                .attach(
                    "a map owns the level below it and nothing further, so this key \
                     belongs to whatever claimed that level",
                ));
        }
        Level::Outside => {
            return Err(Report::new(StorageError::Path)
                .attach(Prefix(path.clone()))
                .attach(RawKey(stored.to_owned()))
                .attach("the key is not under the map it was scanned from"));
        }
    };

    let key = K::from_str(name).map_err(|_| {
        Report::new(StorageError::Codec)
            .attach(Prefix(path.clone()))
            .attach(Entry(name.to_owned()))
            .attach(format!("key type: {}", std::any::type_name::<K>()))
    })?;

    let value = store
        .decode::<V>(bytes)
        .attach_prefix(path)
        .attach_entry(name)?;

    Ok(Some((key, value)))
}

pub fn reactive_map_with_path_only<K, V>(
    store: &Store,
    path: impl IntoStorePath,
    defaults: HashMap<K, V>,
    instance_id: Uuid,
) -> StorageResult<ReactiveMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let path = crate::store::to_path(path)?;
    claim(store, &path, instance_id)?;

    let mut known_cache = load_map::<K, V>(store, &path)?;

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
                    source: event.source,
                });
                return;
            }

            let Some(key_str) = path_for_keys.entry_name(&event.path) else {
                tracing::error!(
                    path = %event.path,
                    map = %path_for_keys,
                    "a key under this map is not a path this library could have written, so the change was not applied"
                );
                return;
            };

            let Ok(k) = K::from_str(&key_str) else {
                tracing::error!(
                    path = %event.path,
                    map = %path_for_keys,
                    key_type = std::any::type_name::<K>(),
                    "a key under this map does not parse as its key type, so the change was not applied"
                );
                return;
            };

            {
                let source = event.source;

                let new_val = match event.new.as_ref().map(|b| store_clone.decode::<V>(b)) {
                    Some(Ok(value)) => Some(value),
                    Some(Err(e)) => {
                        tracing::error!(
                            path = %event.path,
                            "a map entry cannot be read as this map's value type, so the map kept what it had: {e:?}"
                        );
                        return;
                    }
                    None => None,
                };

                let decoded_old = match event.old.as_ref().map(|b| store_clone.decode::<V>(b)) {
                    Some(Ok(value)) => Some(value),
                    Some(Err(e)) => {
                        tracing::warn!(
                            path = %event.path,
                            "the value being replaced could not be read, so subscribers are told what this map had: {e:?}"
                        );
                        None
                    }
                    None => None,
                };

                let old_val =
                    decoded_old.or_else(|| core_clone.cache.get(&k).map(|v| v.clone()));

                let change = {
                    let keys = &core_clone.cache;

                    match event.op {
                        StoreOp::Set => {
                            let Some(new_value) = new_val else {
                                tracing::error!(
                                    path = %event.path,
                                    "a set carried no value, so the map kept what it had"
                                );
                                return;
                            };

                            if keys.contains_key(&k) {
                                let old_value = old_val.unwrap_or_default();
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
                                old_value: old_val.unwrap_or_default(),
                                source,
                            }
                        }
                    }
                };

                core_clone.notify(&change);
            }
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
