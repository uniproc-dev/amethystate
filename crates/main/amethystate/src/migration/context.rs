use crate::codec::CodecError;
use crate::migration::fields::AmeStateFields;
use crate::migration::migrate_from::MigrateFrom;
use crate::migration::provided::Provided;
use crate::store::MigrationBackendAdapter;
use crate::store::facts::{Entry, Facts, Migrating, Prefix, RawKey};
use crate::store::{CodecFormat, StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::any::{Any, type_name};
use std::collections::HashMap;
use std::hash::Hash;
use std::str::FromStr;

pub struct MigrationContext<'a> {
    prefix: String,
    storage: &'a mut dyn MigrationBackendAdapter,
    provided: Option<&'a Provided>,
}

impl<'a> MigrationContext<'a> {
    /// Builds a context over one prefix. The engine does this; a migration
    /// step receives the result.
    pub fn new(prefix: String, storage: &'a mut dyn MigrationBackendAdapter) -> Self {
        Self {
            prefix,
            storage,
            provided: None,
        }
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
    pub fn require<T: Any>(&self) -> StorageResult<&'a T> {
        if let Some(value) = self.provided::<T>() {
            return Ok(value);
        }

        let offered = self.provided.map(Provided::type_names).unwrap_or_default();
        let offered = if offered.is_empty() {
            "nothing was provided".to_string()
        } else {
            format!("provided: {}", offered.join(", "))
        };

        Err(Report::new(StorageError::Migrate)
            .attach(Migrating(self.prefix.clone()))
            .attach(format!("no value provided for {}", type_name::<T>()))
            .attach(offered)
            .attach("StoreBuilder::provide hands a value to every migration step"))
    }

    /// Migrates a nested struct held at `key`, running its own
    /// [`MigrateFrom`] and returning the new shape.
    ///
    /// For a field that is itself an `#[amethystate]` struct, so its migration
    /// is written once and reused wherever it is nested.
    pub fn nested<TOld, TNew>(&mut self, key: &str, old_data: TOld) -> StorageResult<TNew>
    where
        TOld: AmeStateFields,
        TNew: MigrateFrom<TOld> + AmeStateFields,
    {
        let mut sub_ctx = self.scoped(key);

        let new_data = TNew::migrate(old_data, &mut sub_ctx)?;

        for old_f in TOld::FIELDS {
            let is_renamed = TNew::RENAMES.iter().any(|(ok, _)| *ok == old_f.name);
            let is_kept = TNew::FIELDS.iter().any(|nf| nf.name == old_f.name);

            if is_renamed || !is_kept {
                sub_ctx.delete(old_f.name)?;
            }
        }
        Ok(new_data)
    }

    /// Removes a key that the new schema no longer has.
    ///
    /// Deleting a key that was never there is not an error - a migration has
    /// to survive running against data that skipped a version.
    pub fn delete(&mut self, key: &str) -> StorageResult<()> {
        let scoped = self.scoped_path(key);
        self.storage
            .delete(&scoped)
            .attach_migrating(&self.prefix)
            .attach_raw_key(&scoped)
    }

    /// Moves a value to another key, bytes untouched.
    ///
    /// Nothing is decoded, so this works whatever the value's type and cannot
    /// fail on a type it does not know. A `from` that holds nothing is a
    /// no-op rather than an error.
    pub fn rename(&mut self, from: &str, to: &str) -> StorageResult<()> {
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
        f: impl FnOnce(TOld) -> StorageResult<TNew>,
    ) -> StorageResult<()>
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
        f: impl FnOnce(TOld1, TOld2) -> StorageResult<TNew>,
    ) -> StorageResult<()>
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
        f: impl FnOnce(TOld) -> StorageResult<(TNew1, TNew2)>,
    ) -> StorageResult<()>
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
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> StorageResult<Option<T>> {
        match self.get_raw(key)? {
            Some(bytes) => Ok(Some(
                decode(self.storage, &bytes)
                    .attach_migrating(&self.prefix)
                    .attach_entry(key)
                    .attach_with(|| format!("as: {}", std::any::type_name::<T>()))?,
            )),
            None => Ok(None),
        }
    }

    /// Writes a value relative to this context's prefix.
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) -> StorageResult<()> {
        let bytes = encode(self.storage, value)
            .attach_migrating(&self.prefix)
            .attach_entry(key)
            .attach_with(|| format!("as: {}", std::any::type_name::<T>()))?;
        self.set_raw(key, &bytes)
    }

    /// Reads a value by its whole path, ignoring this context's prefix.
    ///
    /// For a step that needs something another part of the store owns. Whether
    /// that value has already been migrated depends on ordering - declare it
    /// with [`PrefixMigrationBuilder::depends_on`](crate::migration::builder::PrefixMigrationBuilder::depends_on)
    /// rather than hoping.
    pub fn global_get<T: DeserializeOwned>(&self, full_key: &str) -> StorageResult<Option<T>> {
        let read = self
            .storage
            .get(full_key)
            .attach_migrating(&self.prefix)
            .attach_raw_key(full_key)?;

        match read {
            Some(bytes) => Ok(Some(
                decode(self.storage, &bytes)
                    .attach_migrating(&self.prefix)
                    .attach_raw_key(full_key)?,
            )),
            None => Ok(None),
        }
    }

    /// Writes a value by its whole path, ignoring this context's prefix.
    pub fn global_set<T: Serialize>(&mut self, full_key: &str, value: &T) -> StorageResult<()> {
        let bytes = encode(self.storage, value)
            .attach_migrating(&self.prefix)
            .attach_raw_key(full_key)?;
        self.storage
            .set(full_key, &bytes)
            .attach_migrating(&self.prefix)
            .attach_raw_key(full_key)
            .attach_value_bytes(bytes.len())
    }

    /// The stored bytes at `key`, undecoded.
    ///
    /// For moving a value whose type this step cannot name, or reading one
    /// written in a shape that no longer deserialises.
    pub fn get_raw(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let scoped = self.scoped_path(key);
        self.storage
            .get(&scoped)
            .attach_migrating(&self.prefix)
            .attach_raw_key(&scoped)
    }

    /// Writes bytes at `key` as they are.
    ///
    /// They must be in the backend's own encoding - [`encode`] produces it.
    pub fn set_raw(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        let scoped = self.scoped_path(key);
        self.storage
            .set(&scoped, value)
            .attach_migrating(&self.prefix)
            .attach_raw_key(&scoped)
            .attach_value_bytes(value.len())
    }

    /// A context narrowed to a sub-prefix, so a nested part can be migrated
    /// with keys relative to it.
    pub fn scoped(&mut self, sub_prefix: &str) -> MigrationContext<'_> {
        MigrationContext {
            prefix: self.scoped_path(sub_prefix),
            storage: self.storage,
            provided: self.provided,
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
    pub fn scan_map<K, V>(&self, key: &str) -> StorageResult<HashMap<K, V>>
    where
        K: FromStr + Eq + Hash,
        V: DeserializeOwned,
    {
        let scoped = self.scoped_path(key);
        let full_prefix = StorePath::parse_joined(&scoped)
            .change_context(StorageError::Path)
            .attach_raw_key(&scoped)?;
        let raw = self
            .storage
            .scan_prefix(&full_prefix)
            .attach_migrating(&self.prefix)
            .attach_prefix(&full_prefix)?;
        let mut map = HashMap::new();

        for (path, bytes) in raw {
            let below = amethystate_core::path::level_under(path.as_str(), &full_prefix)
                .change_context(StorageError::Path)
                .attach_prefix(&full_prefix)
                .attach_raw_key(path.as_str())?;

            let name = match below {
                amethystate_core::path::Level::Entry(name) => name.into_owned(),
                amethystate_core::path::Level::Deeper(name) => {
                    return Err(Report::new(StorageError::Path)
                        .attach(Prefix(full_prefix.clone()))
                        .attach(RawKey(path.to_string()))
                        .attach(Entry(name.into_owned()))
                        .attach(
                            "a map owns the level below it and nothing further, and this \
                             step would rewrite the map whole",
                        ));
                }
                amethystate_core::path::Level::Prefix | amethystate_core::path::Level::Outside => {
                    return Err(Report::new(StorageError::Path)
                        .attach(Prefix(full_prefix.clone()))
                        .attach(RawKey(path.to_string()))
                        .attach("the key is not under the map it was scanned from"));
                }
            };

            let parsed = K::from_str(&name).map_err(|_| {
                Report::new(StorageError::Codec)
                    .attach(Prefix(full_prefix.clone()))
                    .attach(Entry(name.clone()))
                    .attach(format!("key type: {}", std::any::type_name::<K>()))
            })?;

            let value = decode::<V>(self.storage, &bytes)
                .attach_prefix(&full_prefix)
                .attach_entry(&name)
                .attach_with(|| format!("value type: {}", std::any::type_name::<V>()))?;

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

        #[cfg(test)]
        CodecFormat::Default => serde_json::to_vec(value)
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

        #[cfg(test)]
        CodecFormat::Default => serde_json::from_slice(bytes)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec),
    }
}

#[cfg(test)]
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
            CodecFormat::Default
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

        fn get_schema_snapshot(
            &self,
            _prefix: &StorePath,
        ) -> StorageResult<Option<SchemaSnapshot>> {
            unreachable!()
        }

        fn set_schema_snapshot(
            &mut self,
            _prefix: &StorePath,
            _snapshot: &SchemaSnapshot,
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
