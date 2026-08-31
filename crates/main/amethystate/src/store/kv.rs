use crate::migration::fields::{FieldDescriptor, Role};
use crate::observability::SchemaEntry;
use crate::observability::register_instance;
use crate::reactive::error::{WriteError, WriteResult};
use crate::store::Durable;
use crate::store::facts::Facts;
use crate::store::{
    InitState, StorageResult, StoreBackend, field_with_path, reactive_map_with_path_only,
};
use crate::{ReactiveCell, ReactiveMap, Store};
use crate::{ReactiveMapKey, ReactiveMapValue};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Reactive values addressed by path, without declaring a struct.
///
/// For values whose set is not known at compile time, or where a schema is more
/// ceremony than the job is worth. Nothing here is versioned or migrated, and
/// drift is not tracked - that is what the typed structs are for.
///
/// What comes back is an ordinary [`ReactiveCell`] or [`ReactiveMap`], so
/// subscriptions and local delivery work exactly as they do for declared
/// fields. Only the addressing differs.
pub struct Kv {
    store: Store,
    instance_id: Uuid,
    prefix: Option<StorePath>,
}

impl Kv {
    pub(crate) fn new(store: Store) -> Self {
        let instance_id = Uuid::new_v4();
        register_instance(instance_id, "amethystate::Kv");

        Self {
            store,
            instance_id,
            prefix: None,
        }
    }

    /// A handle on everything under `name`.
    ///
    /// Nesting is spelled here, so a name stays a name: `namespace("ui")` then
    /// `set("dark.mode", ..)` addresses one value called `dark.mode` under
    /// `ui`. The namespace's own name is read the same way: a separator in it
    /// is part of it.
    ///
    /// A namespace is a view on the same `Kv`, so writes through it carry the
    /// provenance of the handle it came from.
    ///
    /// # Panics
    ///
    /// If `name` is empty, which is not a level and so cannot be a namespace.
    /// A namespace is written out in the source, where an empty one is a typo
    /// rather than a condition to handle; [`Kv::try_namespace`] is for a name
    /// that comes from data.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let ui = store.kv().namespace("ui");
    ///
    /// ui.set("width", &1280u32).unwrap();
    /// assert_eq!(ui.get::<u32>("width").unwrap(), Some(1280));
    ///
    /// // The same value, spelled out from the root.
    /// assert_eq!(store.kv().namespace("ui").get::<u32>("width").unwrap(), Some(1280));
    ///
    /// // Namespaces nest.
    /// let panels = ui.namespace("panels");
    /// panels.set("left", &true).unwrap();
    /// assert_eq!(ui.namespace("panels").get::<bool>("left").unwrap(), Some(true));
    ///
    /// // A separator in the namespace's name is part of the name, so this is
    /// // one level called `a.b` rather than a level `b` under a level `a`.
    /// let odd = store.kv().namespace("a.b");
    /// odd.set("x", &1u32).unwrap();
    /// assert_eq!(odd.get::<u32>("x").unwrap(), Some(1));
    /// assert_eq!(store.kv().namespace("a").get::<u32>("x").unwrap(), None);
    /// ```
    #[track_caller]
    pub fn namespace(&self, name: &str) -> Self {
        self.try_namespace(name)
            .expect("a namespace name cannot be empty")
    }

    /// [`Kv::namespace`] for a name that can turn out to be empty.
    pub fn try_namespace(&self, name: &str) -> WriteResult<Self> {
        Ok(Self {
            store: self.store.clone(),
            instance_id: self.instance_id,
            prefix: Some(self.resolve(name)?),
        })
    }

    /// Where this handle is rooted, or `None` at the top.
    pub fn prefix(&self) -> Option<&StorePath> {
        self.prefix.as_ref()
    }

    fn resolve(&self, name: &str) -> WriteResult<StorePath> {
        match &self.prefix {
            Some(prefix) => prefix.try_push(name),
            None => StorePath::try_segment(name),
        }
        .change_context(WriteError::Path)
        .attach_with(|| match &self.prefix {
            Some(prefix) => format!("namespace: {prefix}, name: {name}"),
            None => format!("name: {name}"),
        })
    }

    /// Reads a value, or `None` if the path holds nothing.
    ///
    /// Raw: the type is whatever you ask for here. Nothing records it, and
    /// nothing checks it against what an earlier run wrote - if the bytes do
    /// not fit, the read says so and names what it found.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let kv = store.kv();
    ///
    /// assert_eq!(kv.get::<u32>("width").unwrap(), None);
    /// kv.set("width", &1280u32).unwrap();
    /// assert_eq!(kv.get::<u32>("width").unwrap(), Some(1280));
    /// ```
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> WriteResult<Option<T>> {
        let path = self.resolve(name)?;
        self.store
            .get(&path)
            .change_context(WriteError::Storage)
            .attach_key(&path)
    }

    /// The same writes, each returning only once the change is on disk.
    pub fn durable(&self) -> Durable<'_, Self> {
        Durable(self)
    }

    /// Writes a value at `path`, creating it or replacing what was there.
    ///
    /// The write is buffered and flushed on the store's own schedule;
    /// [`Kv::durable`] is the form that returns once it is on disk.
    ///
    /// `Kv` is addressed by path and has no notion of a key that must already
    /// exist, so every write here creates as readily as it replaces.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let kv = store.kv();
    ///
    /// kv.set("theme", &"dark".to_string()).unwrap();
    /// assert_eq!(kv.get::<String>("theme").unwrap(), Some("dark".into()));
    ///
    /// // A name is a name: a separator in one is part of it, and `namespace`
    /// // is how a level is asked for.
    /// kv.set("ui.theme", &"solarized".to_string()).unwrap();
    /// assert_eq!(kv.get::<String>("ui.theme").unwrap(), Some("solarized".into()));
    /// assert_eq!(kv.namespace("ui").get::<String>("theme").unwrap(), None);
    /// ```
    pub fn set<T: Serialize>(&self, name: &str, value: &T) -> WriteResult<()> {
        let path = self.resolve(name)?;
        self.guard(&path)?;
        self.store
            .set_with_source(&path, value, Some(self.instance_id))
            .change_context(WriteError::Storage)
            .attach_key(&path)?;
        Ok(())
    }

    /// Drops whatever is at `path`. Removing an absent path succeeds.
    ///
    /// The removal is buffered and flushed on the store's own schedule;
    /// [`Kv::durable`] is the form that returns once it is on disk.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let kv = store.kv();
    ///
    /// kv.set("theme", &"dark".to_string()).unwrap();
    /// kv.remove("theme").unwrap();
    /// assert_eq!(kv.get::<String>("theme").unwrap(), None);
    ///
    /// // Again, on a name that now holds nothing.
    /// kv.remove("theme").unwrap();
    /// ```
    pub fn remove(&self, name: &str) -> WriteResult<()> {
        let path = self.resolve(name)?;
        self.guard(&path)?;
        self.store
            .delete_with_source(&path, Some(self.instance_id))
            .change_context(WriteError::Storage)
            .attach_key(&path)?;
        Ok(())
    }

    /// Every path under `prefix`, sorted, without reading the values.
    ///
    /// The paths come back whole, prefix included - not as the remainder
    /// after it.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let kv = store.kv();
    ///
    /// let ui = kv.namespace("ui");
    /// ui.set("theme", &"dark".to_string()).unwrap();
    /// ui.set("width", &1280u32).unwrap();
    /// kv.namespace("net").set("port", &8080u16).unwrap();
    ///
    /// assert_eq!(
    ///     ui.keys().unwrap().iter().map(|p| p.as_str()).collect::<Vec<_>>(),
    ///     ["ui.theme", "ui.width"]
    /// );
    /// ```
    #[doc = include_str!("scan_contract.md")]
    pub fn keys(&self) -> StorageResult<Vec<StorePath>> {
        match &self.prefix {
            Some(prefix) => self.store.scan_keys(prefix),
            None => self.store.scan_keys(StorePath::root()),
        }
    }

    /// A reactive cell over one path, seeded with `default` if the path is
    /// empty.
    ///
    /// A cell reads the path to seed itself, so asking for the same path as two
    /// different types fails rather than handing back garbage - the second ask
    /// finds what the first stored and says what it found.
    ///
    /// The cell owns the field behind it, since nothing else holds one: the
    /// field exists because the cell was asked for. So it stays readable and
    /// writable for as long as it is held - and keeps the store open for that
    /// long - where [`Field::cell`](crate::Field::cell) is a view that empties
    /// when its field is dropped.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let kv = store.kv();
    ///
    /// let theme = kv.cell("theme", "dark".to_string()).unwrap();
    /// assert_eq!(theme.get(), Some("dark".to_string()));
    ///
    /// theme.set("light".to_string()).unwrap();
    /// assert_eq!(kv.get::<String>("theme").unwrap(), Some("light".into()));
    ///
    /// // A second type for the same path is refused - by the read, which finds
    /// // a string where a number was asked for.
    /// assert!(kv.cell("theme", 0u32).is_err());
    /// ```
    pub fn cell<T>(&self, name: &str, default: T) -> WriteResult<ReactiveCell<T>>
    where
        T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
    {
        let path = self.resolve(name)?;
        self.guard(&path)?;

        let field = field_with_path::<T>(&self.store, path.clone(), default, self.instance_id)
            .change_context(WriteError::Storage)
            .attach_key(&path)?;

        let cell = field.cell();
        Ok(cell.owning(Arc::new(field)))
    }

    /// A reactive map under `name`, for a key set that is not known up front.
    ///
    /// Everything a declared `ReactiveMap` field can do, without declaring a
    /// struct - subscriptions, interceptors and durable writes included.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let kv = store.kv();
    ///
    /// let widths = kv.map::<String, u64>("columns").unwrap();
    /// widths.insert("cpu".into(), &120).unwrap();
    ///
    /// // A map is a namespace with entries in it.
    /// let columns = kv.namespace("columns");
    /// assert_eq!(columns.get::<u64>("cpu").unwrap(), Some(120));
    /// ```
    pub fn map<K, V>(&self, name: &str) -> WriteResult<ReactiveMap<K, V>>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        let path = self.resolve(name)?;
        self.guard(&path)?;

        reactive_map_with_path_only(&self.store, path.clone(), HashMap::new(), self.instance_id)
            .change_context(WriteError::Storage)
            .attach_prefix(&path)
    }

    /// Refuses a path a declared struct owns.
    ///
    /// Writing a `String` where a `u16` is declared does not merely store the
    /// wrong thing: the field's subscription fails to decode and keeps its old
    /// value, and the next startup fails outright when it reads the path back.
    ///
    /// What is owned is the declared path and whatever lies inside it. The
    /// prefix it sits under stays open, so an extension, a theme or a person
    /// editing the file can put their own keys beside the declared ones:
    /// `app.width` is owned, and `app.myplugin.enabled` is nobody's.
    fn guard(&self, path: &StorePath) -> WriteResult<()> {
        let Some((found, struct_name)) = schema_collision(path) else {
            return Ok(());
        };

        let declared = match found {
            Collision::Owned(declared) | Collision::Holds(declared) => declared,
        };

        Err(Report::new(WriteError::SchemaOwned {
            path: path.as_str().to_string(),
            declared: declared.as_str().to_string(),
        })
        .attach(format!("declared by: {struct_name}")))
    }

    /// Drops everything under this handle that no schema declared.
    ///
    /// What a plugin, a theme or a person put beside the declared settings goes;
    /// the declared paths stay, and so does anything inside them. A path a
    /// schema declares *under* it is descended into rather than skipped, so an
    /// undeclared key beside a declared one is still removed.
    ///
    /// The declared values are left exactly as they are, and their
    /// initialization markers are untouched; [`Kv::reset_to_defaults`] is the
    /// call that seeds them again.
    ///
    /// Returns what went and what stayed, because a settings screen that resets
    /// something should be able to say what it reset.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let ui = store.kv().namespace("ui");
    /// let plugin = ui.namespace("myplugin");
    ///
    /// ui.set("theme", &"dark".to_string()).unwrap();
    /// plugin.set("enabled", &true).unwrap();
    ///
    /// let cleared = ui.clear().unwrap();
    ///
    /// assert!(cleared.kept.is_empty(), "no schema declares anything under `ui`");
    /// assert!(cleared.removed.iter().any(|p| p.as_str() == "ui.theme"));
    /// assert!(
    ///     cleared.removed.iter().any(|p| p.starts_with(plugin.prefix().unwrap())),
    ///     "the subtree went, at whatever depth this engine names it"
    /// );
    ///
    /// assert_eq!(ui.get::<String>("theme").unwrap(), None);
    /// assert_eq!(plugin.get::<bool>("enabled").unwrap(), None);
    /// ```
    pub fn clear(&self) -> StorageResult<Cleared> {
        let at = self.prefix.clone().unwrap_or_else(StorePath::root);
        let mut cleared = Cleared::default();

        self.clear_under(&at, &mut cleared)?;

        Ok(cleared)
    }

    /// Drops the declared values under this handle so the next construction
    /// writes the defaults again.
    ///
    /// The other half of [`Kv::clear`]: that one keeps the declared paths and
    /// drops everything else, this one drops the declared paths and keeps
    /// everything else.
    ///
    /// A namespace's initialization marker is cleared *before* its values are
    /// dropped, and the order is not a detail. The marker is the only thing that
    /// tells a namespace it has been written before, so dropping the values
    /// first and failing in between leaves them gone and the marker standing -
    /// nothing re-seeds and the settings are lost for good. In this order the
    /// same failure leaves the marker cleared and the values in place, and the
    /// next start finds them, writes nothing, and marks the namespace again.
    pub fn reset_to_defaults(&self) -> StorageResult<Cleared> {
        let at = self.prefix.clone().unwrap_or_else(StorePath::root);
        let mut cleared = Cleared::default();

        for namespace in seeded_namespaces_under(&at) {
            self.store
                .set_initialized(&namespace, InitState::Fresh)?;
        }

        self.reset_under(&at, &mut cleared)?;

        Ok(cleared)
    }

    fn reset_under(&self, at: &StorePath, cleared: &mut Cleared) -> StorageResult<()> {
        for child in self.store.scan_keys(at)? {
            match schema_collision(&child) {
                Some((Collision::Owned(_), _)) => {
                    self.store
                        .delete_prefix_with_source(&child, Some(self.instance_id))?;
                    cleared.removed.push(child);
                }
                Some((Collision::Holds(_), _)) => self.reset_under(&child, cleared)?,
                None => cleared.kept.push(child),
            }
        }

        Ok(())
    }

    fn clear_under(&self, at: &StorePath, cleared: &mut Cleared) -> StorageResult<()> {
        for child in self.store.scan_keys(at)? {
            match schema_collision(&child) {
                None => {
                    self.store
                        .delete_prefix_with_source(&child, Some(self.instance_id))?;
                    cleared.removed.push(child);
                }
                Some((Collision::Holds(_), _)) => self.clear_under(&child, cleared)?,
                Some((Collision::Owned(_), _)) => cleared.kept.push(child),
            }
        }

        Ok(())
    }
}

/// What [`Kv::clear`] did.
///
/// How deep the paths are follows the engine's scan - see
/// [`StoreBackend::scan_keys`] - so a flat engine names the leaves it removed
/// and a document engine names the level it removed them with. What went is the
/// same either way.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cleared {
    /// Paths that were removed, with everything under them.
    pub removed: Vec<StorePath>,

    /// Paths left alone because a schema declares them.
    pub kept: Vec<StorePath>,
}

/// How a path meets what a schema declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collision {
    /// The path is a declared one, or lies inside one - a field owns whatever
    /// is under it, since that is the inside of its value, and a map owns its
    /// entries.
    Owned(StorePath),

    /// A declared path lies under this one, so a value here, or a map, would
    /// take the level those paths live on.
    Holds(StorePath),
}

/// How `path` meets the paths a schema declared, if it meets them at all.
///
/// A node holds nothing itself and is only the way to the paths below it, so
/// `app.panel` meets a schema through its children, as a
/// [`Collision::Holds`] naming one of them.
fn collision(at: &StorePath, fields: &[FieldDescriptor], path: &StorePath) -> Option<Collision> {
    for field in fields {
        let Ok(declared) = at.try_push(field.name) else {
            continue;
        };

        if declared.starts_with(path) && &declared != path {
            return Some(Collision::Holds(declared));
        }

        match field.role {
            Role::Node => {
                if path.starts_with(&declared)
                    && let Some(found) = collision(&declared, field.children, path)
                {
                    return Some(found);
                }
            }
            Role::Field | Role::Map => {
                if path.starts_with(&declared) {
                    return Some(Collision::Owned(declared));
                }
            }
        }
    }

    None
}

/// Every path at or under `at` that a construction marks as seeded.
///
/// A struct marks its own prefix, a nested node marks the path it was built at,
/// and a map marks its own path - so the set is the prefix, every `Role::Node`
/// under it, and every `Role::Map`.
fn seeded_namespaces_under(at: &StorePath) -> Vec<StorePath> {
    let mut found = Vec::new();

    for entry in inventory::iter::<SchemaEntry> {
        let Some(prefix) = &entry.prefix else {
            continue;
        };
        if !prefix.starts_with(at) {
            continue;
        }

        found.push(prefix.clone());
        collect_seeded(prefix, entry.fields, &mut found);
    }

    found
}

fn collect_seeded(at: &StorePath, fields: &[FieldDescriptor], found: &mut Vec<StorePath>) {
    for field in fields {
        let Ok(path) = at.try_push(field.name) else {
            continue;
        };

        match field.role {
            Role::Node => {
                collect_seeded(&path, field.children, found);
                found.push(path);
            }
            Role::Map => found.push(path),
            Role::Field => {}
        }
    }
}

/// How `path` meets every declared schema, if it meets any.
fn schema_collision(path: &StorePath) -> Option<(Collision, &'static str)> {
    for entry in inventory::iter::<SchemaEntry> {
        let Some(prefix) = &entry.prefix else {
            continue;
        };
        if !path.starts_with(prefix) && !prefix.starts_with(path) {
            continue;
        }

        if let Some(found) = collision(prefix, entry.fields, path) {
            return Some((found, entry.struct_name));
        }
    }

    None
}

impl Durable<'_, Kv> {
    /// Writes a value at `path`, creating it or replacing what was there.
    ///
    /// Returns only once it is on disk rather than buffered.
    pub fn set<T: Serialize>(&self, name: &str, value: &T) -> WriteResult<()> {
        self.0.set(name, value)?;
        let path = self.0.resolve(name)?;
        self.0
            .store
            .flush_prefix(&path)
            .change_context(WriteError::Storage)
            .attach_key(&path)?;
        Ok(())
    }

    /// Writes a value at `path`, creating it or replacing what was there.
    ///
    /// Resolves once the change is on disk. Like every future, this does
    /// nothing until awaited - the write included.
    pub async fn set_async<T: Serialize>(&self, name: &str, value: &T) -> WriteResult<()> {
        self.0.set(name, value)?;
        let path = self.0.resolve(name)?;
        self.0
            .store
            .flush_async()
            .await
            .change_context(WriteError::Storage)
            .attach_key(&path)?;
        Ok(())
    }

    /// Drops whatever is at `path`.
    ///
    /// Returns only once the removal is on disk rather than buffered.
    pub fn remove(&self, name: &str) -> WriteResult<()> {
        self.0.remove(name)?;
        let path = self.0.resolve(name)?;
        self.0
            .store
            .flush_prefix(&path)
            .change_context(WriteError::Storage)
            .attach_key(&path)?;
        Ok(())
    }

    /// Drops whatever is at `path`.
    ///
    /// Resolves once the change is on disk. Like every future, this does
    /// nothing until awaited - the removal included.
    pub async fn remove_async(&self, name: &str) -> WriteResult<()> {
        self.0.remove(name)?;
        let path = self.0.resolve(name)?;
        self.0
            .store
            .flush_async()
            .await
            .change_context(WriteError::Storage)
            .attach_key(&path)?;
        Ok(())
    }
}
