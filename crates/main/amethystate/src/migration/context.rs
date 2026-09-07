use crate::codec::CodecError;
use crate::migration::fields::{AmeStateFields, FieldDescriptor, Role};
use crate::migration::migrate_from::MigrateFrom;
use crate::migration::provided::Provided;
use crate::migration::step::{RunStep, StepResult};
use crate::store::MigrationBackendAdapter;
use crate::store::facts::{Entry, Facts, Prefix, RawKey};
use crate::store::{CodecFormat, StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::any::{Any, type_name};
use std::hash::Hash;
use std::str::FromStr;
use std::sync::Arc;

/// Brings the prefix a step is reaching into up to date before it is read.
///
/// A step that stays inside its own prefix needs no ordering: the engine is
/// already there. A step that reaches out is the only thing that can want
/// another prefix migrated first, and reaching out is a thing it does rather
/// than a thing it declares - so this is asked at the moment of the reach, and
/// the engine answers it by migrating that prefix on the spot.
///
/// Implemented by the engine's pass. A context built without one - a test with
/// a hand-made storage - reaches whatever is on disk, which is what it asked
/// for.
pub trait Reaching {
    /// Migrates whatever prefix `full_key` falls under, unless it is already
    /// done or already running.
    ///
    /// A prefix already running is a cycle, and comes back named end to end.
    fn reach(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        from: &str,
        full_key: &str,
    ) -> StorageResult<()>;
}

pub struct MigrationContext<'a> {
    prefix: String,
    storage: &'a mut dyn MigrationBackendAdapter,
    provided: Option<&'a Provided>,
    reaching: Option<&'a dyn Reaching>,
}

impl<'a> MigrationContext<'a> {
    /// Builds a context over one prefix. The engine does this; a migration
    /// step receives the result.
    pub fn new(prefix: String, storage: &'a mut dyn MigrationBackendAdapter) -> Self {
        Self {
            prefix,
            storage,
            provided: None,
            reaching: None,
        }
    }

    /// Lends the pass that can bring another prefix up to date. See
    /// [`Reaching`].
    pub fn with_reaching(mut self, reaching: &'a dyn Reaching) -> Self {
        self.reaching = Some(reaching);
        self
    }

    /// Lends the values the application handed to
    /// [`StoreBuilder::provide`](crate::StoreBuilder::provide).
    pub fn with_provided(mut self, provided: &'a Provided) -> Self {
        self.provided = Some(provided);
        self
    }

    /// A value the application provided, or `None` if it did not.
    ///
    /// A step is a bare `fn` and captures nothing, so this is how anything
    /// from outside the store reaches it. Use [`MigrationContext::require`]
    /// where the step cannot do its job without it.
    ///
    /// The borrow is on the provided values rather than on the context, so a
    /// step can hold one and go on writing - `ctx.set` while a provided value
    /// is in hand is the ordinary shape of a step, not a fight with the
    /// borrow checker.
    pub fn provided<T: Any>(&self) -> Option<&'a T> {
        self.provided.and_then(Provided::get::<T>)
    }

    /// The same, as a failure rather than a `None`.
    ///
    /// A missing dependency is a wiring mistake in the application, not bad
    /// data, and it is worth saying so plainly: the report names the type the
    /// step asked for and lists what was actually on offer, because the usual
    /// cause is providing a `Foo` where the step wanted an `Arc<Foo>`.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// // What the application knows and the store does not.
    /// struct LegacyDefaults {
    ///     port: u16,
    /// }
    ///
    /// let (store, report) = StoreBuilder::new(&*path)
    ///     .provide(LegacyDefaults { port: 8080 })
    ///     .migrations(|m| {
    ///         m.for_prefix("net").step(1, "carry the old port over", |ctx| {
    ///             let legacy = ctx.require::<LegacyDefaults>()?;
    ///             ctx.set("port", &legacy.port)
    ///         });
    ///     })
    ///     .build_with_migration()
    ///     .unwrap();
    ///
    /// assert!(!report.has_failures());
    /// assert_eq!(store.get::<u16>(["net", "port"]).unwrap(), Some(8080));
    /// ```
    ///
    /// Asking for something nobody provided fails the step, and the report
    /// names the type rather than reading as bad data:
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// struct NeverProvided;
    ///
    /// let (_store, report) = StoreBuilder::new(&*path)
    ///     .migrations(|m| {
    ///         m.for_prefix("net").step(1, "wants what nobody gave", |ctx| {
    ///             ctx.require::<NeverProvided>()?;
    ///             Ok(())
    ///         });
    ///     })
    ///     .build_with_migration()
    ///     .unwrap();
    ///
    /// assert!(report.has_failures());
    ///
    /// let rendered = format!("{report:?}");
    /// assert!(rendered.contains("NeverProvided"));
    ///
    /// // What the report says is pinned by an insta snapshot over in
    /// // `tests/migration_provided.rs`. This reads the same file rather than
    /// // quoting it, so rewording the guidance moves both together or fails
    /// // here - the two cannot drift apart while nobody is looking.
    /// let pinned = include_str!(
    ///     "../../tests/snapshots/migration_provided__migration_wants_a_value_nobody_provided.snap"
    /// );
    /// let guidance = pinned.lines().last().unwrap().trim_start_matches(['╰', '╴']);
    /// assert!(rendered.contains(guidance), "{rendered}");
    /// ```
    pub fn require<T: Any>(&self) -> StepResult<&'a T> {
        if let Some(value) = self.provided::<T>() {
            return Ok(value);
        }

        let offered = self.provided.map(Provided::type_names).unwrap_or_default();
        let offered = if offered.is_empty() {
            "nothing was provided".to_string()
        } else {
            format!("provided: {}", offered.join(", "))
        };

        Err(RunStep::NothingProvided {
            under: Arc::from(self.prefix.as_str()),
            wanted: type_name::<T>(),
            on_offer: Arc::from(offered.as_str()),
        })
    }

    /// Migrates a nested struct held at `key`, running its own
    /// [`MigrateFrom`] and returning the new shape.
    ///
    /// For a field that is itself an `#[amethystate]` struct, so its migration
    /// is written once and reused wherever it is nested.
    pub fn nested<TOld, TNew>(&mut self, key: &str, old_data: TOld) -> StepResult<TNew>
    where
        TOld: AmeStateFields,
        TNew: MigrateFrom<TOld> + AmeStateFields,
    {
        let mut sub_ctx = self.scoped(key);

        let new_data = TNew::migrate(old_data, &mut sub_ctx)?;
        sub_ctx.drop_withdrawn::<TOld, TNew>()?;
        Ok(new_data)
    }

    /// Removes a key that the new schema no longer has.
    ///
    /// Deleting a key that was never there is not an error - a migration has
    /// to survive running against data that skipped a version.
    pub fn delete(&mut self, key: &str) -> StepResult<()> {
        let scoped = self.scoped_path(key);
        self.storage
            .delete(&scoped)
            .attach_migrating(&self.prefix)
            .attach_raw_key(&scoped)
            .map_err(RunStep::Store)
    }

    /// Removes a place and everything under it, for a declaration that owned
    /// more than one key.
    pub fn delete_prefix(&mut self, key: &str) -> StepResult<()> {
        let scoped = self.scoped_path(key);
        let path = StorePath::parse_joined(&scoped)?;
        self.storage
            .delete_prefix(&path)
            .attach_migrating(&self.prefix)
            .attach_prefix(&path)
            .map_err(RunStep::Store)
    }

    /// Removes every place `TOld` declared that `TNew` does not - a field
    /// dropped, or renamed and so read from its old place and written to a new
    /// one.
    ///
    /// What comes off is what the declaration owned: a field is one key, a map
    /// is its entries as well, and a node owns nothing of its own, so what goes
    /// is each of its fields in turn. A node's level is not swept, because a
    /// key written beside its fields belongs to whoever wrote it.
    ///
    /// A `#[rename]` names fields the way the source spells them, so it is
    /// matched against [`declared`](FieldDescriptor::declared) rather than
    /// against the place - the two differ under `path` and `rename_all`.
    pub fn drop_withdrawn<TOld, TNew>(&mut self) -> StepResult<()>
    where
        TOld: AmeStateFields,
        TNew: MigrateFrom<TOld> + AmeStateFields,
    {
        for old_f in TOld::FIELDS {
            let is_renamed = TNew::RENAMES.iter().any(|(ok, _)| *ok == old_f.declared);
            let is_kept = TNew::FIELDS.iter().any(|nf| nf.name == old_f.name);

            if is_renamed || !is_kept {
                self.drop_place(&StorePath::root(), old_f)?;
            }
        }
        Ok(())
    }

    fn drop_place(&mut self, at: &StorePath, field: &FieldDescriptor) -> StepResult<()> {
        match field.owns(at) {
            Some(place) if field.role.same(Role::Map) => self.delete_prefix(place.as_str()),
            Some(place) => self.delete(place.as_str()),
            None => {
                let below = field.below(at);
                for child in field.children {
                    self.drop_place(&below, child)?;
                }
                Ok(())
            }
        }
    }

    /// Moves a value to another key, bytes untouched.
    ///
    /// Nothing is decoded, so this works whatever the value's type and cannot
    /// fail on a type it does not know. A `from` that holds nothing is a
    /// no-op rather than an error.
    pub fn rename(&mut self, from: &str, to: &str) -> StepResult<()> {
        if let Some(bytes) = self.get_raw(from)? {
            self.set_raw(to, &bytes)?;
            self.delete(from)?;
        }
        Ok(())
    }

    /// Reads a value as `TOld`, hands it to `f`, and writes the result back
    /// as `TNew`.
    ///
    /// The one to reach for when a field changes type or representation but
    /// keeps its place.
    pub fn transform<TOld, TNew>(
        &mut self,
        key: &str,
        f: impl FnOnce(TOld) -> StepResult<TNew>,
    ) -> StepResult<()>
    where
        TOld: DeserializeOwned,
        TNew: Serialize,
    {
        if let Some(old_val) = self.get::<TOld>(key)? {
            let new_val = f(old_val)?;
            self.set(key, &new_val)?;
        }
        Ok(())
    }

    /// Folds two keys into one: reads both, hands them to `f`, writes the
    /// result at `into` and drops the sources.
    pub fn merge<TOld1, TOld2, TNew>(
        &mut self,
        from: (&str, &str),
        into: &str,
        f: impl FnOnce(TOld1, TOld2) -> StepResult<TNew>,
    ) -> StepResult<()>
    where
        TOld1: DeserializeOwned,
        TOld2: DeserializeOwned,
        TNew: Serialize,
    {
        if let (Some(v1), Some(v2)) = (self.get::<TOld1>(from.0)?, self.get::<TOld2>(from.1)?) {
            let new_val = f(v1, v2)?;
            self.set(into, &new_val)?;
            self.delete(from.0)?;
            self.delete(from.1)?;
        }
        Ok(())
    }

    /// The inverse of [`MigrationContext::merge`]: reads one key, hands it to
    /// `f`, and writes the pair it returns to two keys, dropping the source.
    pub fn split<TOld, TNew1, TNew2>(
        &mut self,
        from: &str,
        into: (&str, &str),
        f: impl FnOnce(TOld) -> StepResult<(TNew1, TNew2)>,
    ) -> StepResult<()>
    where
        TOld: DeserializeOwned,
        TNew1: Serialize,
        TNew2: Serialize,
    {
        if let Some(old_val) = self.get::<TOld>(from)? {
            let (v1, v2) = f(old_val)?;
            self.set(into.0, &v1)?;
            self.set(into.1, &v2)?;
            self.delete(from)?;
        }
        Ok(())
    }

    /// Reads a value as `T`, relative to this context's prefix.
    ///
    /// The escape hatch for a migration the shaped helpers do not cover.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> StepResult<Option<T>> {
        match self.get_raw(key)? {
            Some(bytes) => Ok(Some(
                decode(self.storage, &bytes)
                    .attach_migrating(&self.prefix)
                    .attach_entry(key)
                    .attach_with(|| format!("as: {}", std::any::type_name::<T>()))
                    .map_err(|why| RunStep::reading::<T>(&self.prefix, key, why))?,
            )),
            None => Ok(None),
        }
    }

    /// Writes a value relative to this context's prefix.
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) -> StepResult<()> {
        let bytes = encode(self.storage, value)
            .attach_migrating(&self.prefix)
            .attach_entry(key)
            .attach_with(|| format!("as: {}", std::any::type_name::<T>()))
            .map_err(|why| RunStep::writing::<T>(&self.prefix, key, why))?;
        self.set_raw(key, &bytes)
    }

    /// Reads a value by its whole path, ignoring this context's prefix.
    ///
    /// For a step that needs something another part of the store owns. That
    /// part is brought up to date first, so what comes back is the migrated
    /// value and not whatever the last version left - the reach is the
    /// ordering, and there is nothing to declare. See [`Reaching`].
    pub fn global_get<T: DeserializeOwned>(&mut self, full_key: &str) -> StepResult<Option<T>> {
        self.reach(full_key)?;

        let read = self
            .storage
            .get(full_key)
            .attach_migrating(&self.prefix)
            .attach_raw_key(full_key)
            .map_err(RunStep::Store)?;

        match read {
            Some(bytes) => Ok(Some(
                decode(self.storage, &bytes)
                    .attach_migrating(&self.prefix)
                    .attach_raw_key(full_key)
                    .map_err(|why| RunStep::reading::<T>(&self.prefix, full_key, why))?,
            )),
            None => Ok(None),
        }
    }

    /// Writes a value by its whole path, ignoring this context's prefix.
    ///
    /// The part of the store being written into is brought up to date first,
    /// for the same reason as [`MigrationContext::global_get`]: a value left
    /// where an old version put it would otherwise be migrated after this
    /// write and carried off with the rest.
    pub fn global_set<T: Serialize>(&mut self, full_key: &str, value: &T) -> StepResult<()> {
        self.reach(full_key)?;

        let bytes = encode(self.storage, value)
            .attach_migrating(&self.prefix)
            .attach_raw_key(full_key)
            .map_err(|why| RunStep::writing::<T>(&self.prefix, full_key, why))?;
        self.storage
            .set(full_key, &bytes)
            .attach_migrating(&self.prefix)
            .attach_raw_key(full_key)
            .attach_value_bytes(bytes.len())
            .map_err(RunStep::Store)
    }

    fn reach(&mut self, full_key: &str) -> StepResult<()> {
        let Some(reaching) = self.reaching else {
            return Ok(());
        };

        reaching
            .reach(&mut *self.storage, &self.prefix, full_key)
            .map_err(RunStep::Store)
    }

    /// The stored bytes at `key`, undecoded.
    ///
    /// For moving a value whose type this step cannot name, or reading one
    /// written in a shape that no longer deserialises.
    pub fn get_raw(&self, key: &str) -> StepResult<Option<Vec<u8>>> {
        let scoped = self.scoped_path(key);
        self.storage
            .get(&scoped)
            .attach_migrating(&self.prefix)
            .attach_raw_key(&scoped)
            .map_err(RunStep::Store)
    }

    /// Writes bytes at `key` as they are.
    ///
    /// They must be in the backend's own encoding - [`encode`] produces it.
    pub fn set_raw(&mut self, key: &str, value: &[u8]) -> StepResult<()> {
        let scoped = self.scoped_path(key);
        self.storage
            .set(&scoped, value)
            .attach_migrating(&self.prefix)
            .attach_raw_key(&scoped)
            .attach_value_bytes(value.len())
            .map_err(RunStep::Store)
    }

    /// A context narrowed to a sub-prefix, so a nested part can be migrated
    /// with keys relative to it.
    pub fn scoped(&mut self, sub_prefix: &str) -> MigrationContext<'_> {
        MigrationContext {
            prefix: self.scoped_path(sub_prefix),
            storage: self.storage,
            provided: self.provided,
            reaching: self.reaching,
        }
    }

    /// The same context again, for a part whose keys sit at this level rather
    /// than under one of its own - a node flattened into its holder.
    pub fn here(&mut self) -> MigrationContext<'_> {
        MigrationContext {
            prefix: self.prefix.clone(),
            storage: self.storage,
            provided: self.provided,
            reaching: self.reaching,
        }
    }

    /// Reads a whole [`ReactiveMap`](crate::ReactiveMap) at `key` as a plain
    /// map, so a step can rewrite its entries.
    ///
    /// Every entry under the prefix has to come back. A step reads the map,
    /// changes it and writes it whole, so an entry dropped here is an entry the
    /// migration deletes - and a migration runs once, against data that has no
    /// other copy. An entry that cannot be read is an error, and the
    /// transaction it is in rolls back.
    ///
    /// Filled in the order the scan hands the keys back, which is the order a
    /// `ReactiveMap` walks in and lexicographic on the stored name rather than
    /// on `K` - `10, 100, 9` for numeric keys. A step that goes through the
    /// entries sees what the map itself would show. Writing them back is
    /// per-entry, so what the step does to this order reaches nothing.
    pub fn scan_map<K, V>(&self, key: &str) -> StepResult<IndexMap<K, V>>
    where
        K: FromStr + Eq + Hash,
        V: DeserializeOwned,
    {
        let scoped = self.scoped_path(key);
        let full_prefix = StorePath::parse_joined(&scoped)?;
        let raw = self
            .storage
            .scan_prefix(&full_prefix)
            .attach_migrating(&self.prefix)
            .attach_prefix(&full_prefix)
            .map_err(RunStep::Store)?;
        let mut map = IndexMap::new();

        for (path, bytes) in raw {
            let below = path.level_under(&full_prefix);

            let name = match below {
                amethystate_core::path::Level::Entry(name) => name.into_owned(),
                amethystate_core::path::Level::Deeper(name) => {
                    return Err(RunStep::Store(
                        Report::new(StorageError::Path)
                            .attach(Prefix(full_prefix.clone()))
                            .attach(RawKey(path.to_string()))
                            .attach(Entry(name.into_owned()))
                            .attach(
                                "a map owns the level below it and nothing further, and this \
                                 step would rewrite the map whole",
                            ),
                    ));
                }
                // A map's entries are the level below it and nothing is stored
                // at the path itself, so a scan that hands the path back is
                // reporting the level rather than an entry. A document engine
                // does that for a map somebody emptied, where the level stands
                // with nothing in it.
                amethystate_core::path::Level::Prefix => continue,
                amethystate_core::path::Level::Outside => {
                    return Err(RunStep::Store(
                        Report::new(StorageError::Path)
                            .attach(Prefix(full_prefix.clone()))
                            .attach(RawKey(path.to_string()))
                            .attach("the key is not under the map it was scanned from"),
                    ));
                }
            };

            let parsed = K::from_str(&name).map_err(|_| RunStep::WillNotRead {
                under: Arc::from(full_prefix.as_str()),
                entry: Arc::from(name.as_str()),
                wanted: std::any::type_name::<K>(),
                why: Report::new(StorageError::Codec)
                    .attach(Prefix(full_prefix.clone()))
                    .attach(Entry(name.clone())),
            })?;

            let value = decode::<V>(self.storage, &bytes)
                .attach_prefix(&full_prefix)
                .attach_entry(&name)
                .attach_with(|| format!("value type: {}", std::any::type_name::<V>()))
                .map_err(|why| RunStep::reading::<V>(full_prefix.as_str(), &name, why))?;

            map.insert(parsed, value);
        }

        Ok(map)
    }

    fn scoped_path(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}.{}", self.prefix, key)
        }
    }
}

/// Serialises a value in the backend's own format, for handing to
/// [`MigrationContext::set_raw`].
pub fn encode<T: Serialize>(
    storage: &dyn MigrationBackendAdapter,
    value: &T,
) -> StorageResult<Vec<u8>> {
    match storage.format() {
        #[cfg(feature = "redb")]
        CodecFormat::MessagePack => rmp_serde::to_vec_named(value)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),

        #[cfg(feature = "json")]
        CodecFormat::Json => serde_json::to_vec(value)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),

        #[cfg(feature = "toml")]
        CodecFormat::Toml => {
            #[derive(serde::Serialize)]
            struct Wrap<'a, T> {
                val: &'a T,
            }
            toml_edit::ser::to_string(&Wrap { val: value })
                .map(|s| s.into_bytes())
                .map_err(|e| CodecError::Toml(e.to_string()))
                .change_context(StorageError::Codec)
        }
        #[cfg(feature = "sqlite")]
        CodecFormat::SonicJson => sonic_rs::to_vec(value)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),

        #[cfg(feature = "ron")]
        CodecFormat::Ron => ron::to_string(value)
            .map(|s| s.into_bytes())
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),
    }
}

pub fn decode<T: DeserializeOwned>(
    storage: &dyn MigrationBackendAdapter,
    bytes: &[u8],
) -> StorageResult<T> {
    match storage.format() {
        #[cfg(feature = "redb")]
        CodecFormat::MessagePack => rmp_serde::from_slice(bytes)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),

        #[cfg(feature = "json")]
        CodecFormat::Json => serde_json::from_slice(bytes)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),

        #[cfg(feature = "toml")]
        CodecFormat::Toml => {
            #[derive(serde::Deserialize)]
            struct Unwrap<T> {
                val: T,
            }
            toml_edit::de::from_slice::<Unwrap<T>>(bytes)
                .map(|unwrapped| unwrapped.val)
                .map_err(|e| CodecError::Toml(e.to_string()))
                .change_context(StorageError::Codec)
        }
        #[cfg(feature = "sqlite")]
        CodecFormat::SonicJson => sonic_rs::from_slice(bytes)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),

        #[cfg(feature = "ron")]
        CodecFormat::Ron => ron::de::from_bytes(bytes)
            .map_err(|e| CodecError::from(e.code))
            .change_context(StorageError::Codec),
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::migration::AppliedStep;
    use crate::store::meta::{PrefixMeta, SchemaSnapshot};
    use std::collections::HashMap;

    struct MemoryStorage {
        data: HashMap<String, Vec<u8>>,
    }

    impl MigrationBackendAdapter for MemoryStorage {
        fn format(&self) -> CodecFormat {
            CodecFormat::Json
        }

        fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
            Ok(self.data.get(key).cloned())
        }
        fn set(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
            self.data.insert(key.to_string(), value.to_vec());
            Ok(())
        }
        fn delete(&mut self, key: &str) -> StorageResult<()> {
            self.data.remove(key);
            Ok(())
        }

        fn scan_prefix(&self, _: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
            unreachable!()
        }

        fn get_meta(&self, _prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
            unreachable!()
        }

        fn set_meta(&mut self, _prefix: &StorePath, _meta: &PrefixMeta) -> StorageResult<()> {
            unreachable!()
        }

        fn get_schema_snapshots(&self, _prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>> {
            unreachable!()
        }

        fn set_schema_snapshots(
            &mut self,
            _prefix: &StorePath,
            _trees: &[SchemaSnapshot],
        ) -> StorageResult<()> {
            unreachable!()
        }

        fn get_migration_log(
            &self,
            _prefix: &StorePath,
        ) -> StorageResult<Option<Vec<AppliedStep>>> {
            unreachable!()
        }

        fn set_migration_log(
            &mut self,
            _prefix: &StorePath,
            _log: &[AppliedStep],
        ) -> StorageResult<()> {
            unreachable!()
        }
    }

    #[test]
    fn test_context_rename() {
        let mut storage = MemoryStorage {
            data: HashMap::new(),
        };
        let mut ctx = MigrationContext::new("p".into(), &mut storage);

        ctx.set("a", &100i32).unwrap();
        ctx.rename("a", "b").unwrap();

        assert_eq!(ctx.get::<i32>("b").unwrap(), Some(100));
        assert!(ctx.get::<i32>("a").unwrap().is_none());
    }

    #[test]
    fn test_context_transform() {
        let mut storage = MemoryStorage {
            data: HashMap::new(),
        };
        let mut ctx = MigrationContext::new("p".into(), &mut storage);

        ctx.set("v", &10i32).unwrap();
        ctx.transform::<i32, i32>("v", |v| Ok(v + 5)).unwrap();

        assert_eq!(ctx.get::<i32>("v").unwrap(), Some(15));
    }

    #[test]
    fn test_context_merge() {
        let mut storage = MemoryStorage {
            data: HashMap::new(),
        };
        let mut ctx = MigrationContext::new("p".into(), &mut storage);

        ctx.set("f", &"a".to_string()).unwrap();
        ctx.set("l", &"b".to_string()).unwrap();

        ctx.merge::<String, String, String>(("f", "l"), "res", |f, l| Ok(format!("{}{}", f, l)))
            .unwrap();

        assert_eq!(ctx.get::<String>("res").unwrap(), Some("ab".into()));
        assert!(ctx.get::<String>("f").unwrap().is_none());
        assert!(ctx.get::<String>("l").unwrap().is_none());
    }

    #[test]
    fn test_context_split() {
        let mut storage = MemoryStorage {
            data: HashMap::new(),
        };
        let mut ctx = MigrationContext::new("p".into(), &mut storage);

        ctx.set("full", &"a:b".to_string()).unwrap();

        ctx.split::<String, String, String>("full", ("p1", "p2"), |s| {
            let mut it = s.split(':');
            Ok((
                it.next().unwrap().to_string(),
                it.next().unwrap().to_string(),
            ))
        })
        .unwrap();

        assert_eq!(ctx.get::<String>("p1").unwrap(), Some("a".into()));
        assert_eq!(ctx.get::<String>("p2").unwrap(), Some("b".into()));
        assert!(ctx.get::<String>("full").unwrap().is_none());
    }

    #[test]
    fn test_global_access() {
        let mut storage = MemoryStorage {
            data: HashMap::new(),
        };
        let mut ctx = MigrationContext::new("scoped".into(), &mut storage);

        ctx.global_set("raw.key", &777u32).unwrap();

        assert!(ctx.get::<u32>("raw.key").unwrap().is_none());
        assert_eq!(ctx.global_get::<u32>("raw.key").unwrap(), Some(777));
    }
}
