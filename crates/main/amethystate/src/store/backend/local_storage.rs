use crate::MigrationReport;
use crate::codec::CodecError;
use crate::migration::AppliedStep;
use crate::migration::engine::{MigrationEngine, StorageProvider};
use crate::migration::set::MigrationSet;
use crate::store::backend::utils;
use crate::store::builder::Backend;
use crate::store::config::StoreConfig;
use crate::store::durable::{Commit, CommitSignal};
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::{Facts, Key};
use crate::store::format::{self, StorageFactSet};
use crate::store::inspector::InspectorBackend;
use crate::store::meta::{PrefixMeta, SchemaSnapshot};
use crate::store::screening::Screening;
use crate::store::traits::{MigrationBackendAdapter, StoreLayout};
use crate::store::{
    CodecFormat, InitState, StoreBackend, StoreCallback, StoreEvent, StoreOp, SubscriptionEntry,
    SubscriptionId, SubscriptionKind, WillNotOpen,
};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use parking_lot::RwLock;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use uuid::Uuid;

/// A store kept in the page's `localStorage`.
///
/// One key per path, holding the value as JSON, and the bookkeeping - the
/// version each line stands at, the schemas recorded, the migration log, the
/// format record, which namespaces were seeded - under keys of its own. A key
/// is itself a path, spelled joined: `amethystate`, the store's name as one
/// level, the kind of key, then the path it is about -
/// `amethystate.settings.v.ui.width`. Two stores share a page without meeting,
/// whatever their names hold.
///
/// The page's storage answers at once and the browser keeps it, so a write
/// goes straight in: there is no buffer and nothing to flush, and a write the
/// browser turns down - a full quota - is turned down to the caller.
///
/// A change another page of the same site makes to these keys reaches
/// subscribers as a change from outside, the way an edit to a file does.
///
/// Outside a page the open is refused.
#[derive(Clone)]
pub struct LocalStorageStore {
    inner: Arc<Inner>,
}

struct Inner {
    keys: Keys,
    subscriptions: RwLock<Vec<SubscriptionEntry>>,
    settled: AtomicU64,
    commits: Arc<CommitSignal>,
    closed: AtomicBool,
    budget: Screening,
}

impl std::fmt::Debug for LocalStorageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalStorageStore")
            .field("under", &self.inner.keys.root)
            .finish_non_exhaustive()
    }
}

const VALUE: &str = "v";
const META: &str = "meta";
const SCHEMAS: &str = "schemas";
const LOG: &str = "log";
const SEEDED: &str = "seeded";
const FORMAT: &str = "format";

/// The path every key of a store named `path` sits under: the store's name is
/// one level of it, whatever it holds.
pub(crate) fn root_of(path: &Path) -> StorePath {
    StorePath::from_segments(["amethystate", &*path.to_string_lossy()])
}

/// A store's keys, as paths below its root: a kind, then the path the kind is
/// about.
#[derive(Clone)]
struct Keys {
    root: StorePath,
}

impl Keys {
    fn at(&self, kind: &str, path: &StorePath) -> String {
        self.root
            .join(&utils::bookkeeping_at(kind, path))
            .to_string()
    }

    fn value(&self, path: &StorePath) -> String {
        self.at(VALUE, path)
    }

    fn about(&self, kind: &str, at: &StorePath) -> String {
        self.at(kind, at)
    }

    fn format(&self) -> String {
        self.at(FORMAT, &StorePath::root())
    }

    /// The kind and the path of a key this store wrote, and `None` for any
    /// other key in the page.
    fn read(&self, key: &str) -> Option<(String, StorePath)> {
        let under = StorePath::parse_joined(key)
            .ok()?
            .strip_prefix(&self.root)?;
        let kind = under.segment_at(0)?.as_str().to_string();
        let path = under.strip_prefix(&StorePath::segment(&kind))?;
        Some((kind, path))
    }

    fn path_of(&self, key: &str) -> Option<StorePath> {
        match self.read(key)? {
            (kind, path) if kind == VALUE => Some(path),
            _ => None,
        }
    }

    fn ours(&self, key: &str) -> bool {
        self.read(key).is_some()
    }

    /// A failure, carrying the store's root as its name.
    fn named<T>(&self, outcome: StorageResult<T>) -> StorageResult<T> {
        outcome.map_err(|why| utils::in_the_file(why, Path::new(&self.root.to_string())))
    }
}

#[cfg(target_arch = "wasm32")]
type Handle = web_sys::Storage;

#[cfg(not(target_arch = "wasm32"))]
enum Handle {}

/// The page's storage, reached for the length of one call.
struct Page(Handle);

#[cfg(target_arch = "wasm32")]
impl Page {
    fn open() -> StorageResult<Self> {
        let window = web_sys::window()
            .ok_or_else(|| Report::new(StorageError::Open).attach("there is no window here"))?;

        match window.local_storage() {
            Ok(Some(storage)) => Ok(Self(storage)),
            Ok(None) => {
                Err(Report::new(StorageError::Open).attach("this page has no localStorage"))
            }
            Err(why) => Err(Report::new(StorageError::Open)
                .attach(format!("{why:?}"))
                .attach("the page refused its localStorage")),
        }
    }

    fn get(&self, key: &str) -> StorageResult<Option<String>> {
        self.0.get_item(key).map_err(|why| {
            Report::new(StorageError::Read)
                .attach(format!("{why:?}"))
                .attach(format!("key: {key}"))
        })
    }

    fn set(&self, key: &str, value: &str) -> StorageResult<()> {
        self.0.set_item(key, value).map_err(|why| {
            Report::new(StorageError::Write)
                .attach(format!("{why:?}"))
                .attach(format!("key: {key}"))
                .attach(
                    "the page's storage turned the write down; a full quota is the usual reason",
                )
        })
    }

    fn remove(&self, key: &str) -> StorageResult<()> {
        self.0.remove_item(key).map_err(|why| {
            Report::new(StorageError::Write)
                .attach(format!("{why:?}"))
                .attach(format!("key: {key}"))
        })
    }

    fn keys(&self) -> StorageResult<Vec<String>> {
        let count = self
            .0
            .length()
            .map_err(|why| Report::new(StorageError::Read).attach(format!("{why:?}")))?;

        let mut keys = Vec::with_capacity(count as usize);
        for at in 0..count {
            if let Some(key) = self
                .0
                .key(at)
                .map_err(|why| Report::new(StorageError::Read).attach(format!("{why:?}")))?
            {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Page {
    fn open() -> StorageResult<Self> {
        Err(Report::new(StorageError::Open)
            .attach("localStorage belongs to a page, and this build does not run in one"))
    }

    fn get(&self, _key: &str) -> StorageResult<Option<String>> {
        match self.0 {}
    }

    fn set(&self, _key: &str, _value: &str) -> StorageResult<()> {
        match self.0 {}
    }

    fn remove(&self, _key: &str) -> StorageResult<()> {
        match self.0 {}
    }

    fn keys(&self) -> StorageResult<Vec<String>> {
        match self.0 {}
    }
}

impl Page {
    fn read<T: DeserializeOwned>(&self, key: &str) -> StorageResult<Option<T>> {
        let Some(text) = self.get(key)? else {
            return Ok(None);
        };

        serde_json::from_str(&text)
            .map(Some)
            .map_err(CodecError::from)
            .change_context(StorageError::Read)
            .attach(format!("key: {key}"))
    }

    fn write<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> StorageResult<()> {
        let text = serde_json::to_string(value)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec)
            .attach(format!("key: {key}"))?;
        self.set(key, &text)
    }

    fn values_under(
        &self,
        keys: &Keys,
        prefix: &StorePath,
    ) -> StorageResult<Vec<(StorePath, String)>> {
        let mut found = Vec::new();
        for key in self.keys()? {
            let Some(path) = keys.path_of(&key) else {
                continue;
            };
            if !path.starts_with(prefix) {
                continue;
            }
            if let Some(text) = self.get(&key)? {
                found.push((path, text));
            }
        }
        found.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(found)
    }

    fn everything_of(&self, keys: &Keys) -> StorageResult<Vec<(String, String)>> {
        let mut found = Vec::new();
        for key in self.keys()? {
            if !keys.ours(&key) {
                continue;
            }
            if let Some(text) = self.get(&key)? {
                found.push((key, text));
            }
        }
        Ok(found)
    }

    fn bookkeeping_reads(&self, keys: &Keys) -> StorageResult<()> {
        for key in self.keys()? {
            match keys.read(&key) {
                Some((kind, _)) if kind != VALUE && kind != SEEDED => {}
                _ => continue,
            }
            let Some(text) = self.get(&key)? else {
                continue;
            };
            serde_json::from_str::<serde_json::Value>(&text)
                .map_err(CodecError::from)
                .change_context(StorageError::Open)
                .attach(format!("key: {key}"))
                .attach("the store's bookkeeping in the page's storage is not JSON")?;
        }
        Ok(())
    }

    fn put_back(&self, keys: &Keys, before: &[(String, String)]) -> StorageResult<()> {
        for key in self.keys()? {
            if keys.ours(&key) {
                self.remove(&key)?;
            }
        }
        for (key, text) in before {
            self.set(key, text)?;
        }
        Ok(())
    }
}

/// Takes every key of a store away, where an open that could not read them was
/// told to start fresh, and says whether it is worth opening again.
fn start_fresh(config: &StoreConfig, keys: &Keys, why: &Report<StorageError>) -> bool {
    if config.will_not_open != WillNotOpen::StartFresh {
        return false;
    }

    tracing::warn!(
        under = %keys.root,
        reason = %crate::store::one_line(why),
        "the store would not open and was told to start fresh, so its keys are being taken \
         away and an empty store opened in their place"
    );

    match Page::open().and_then(|page| page.put_back(keys, &[])) {
        Ok(()) => true,
        Err(failed) => {
            tracing::error!(
                under = %keys.root,
                error = %crate::store::one_line(&failed),
                "starting fresh was asked for and the keys would not go, so the store is \
                 refused as it would have been"
            );
            false
        }
    }
}

fn decode(
    bytes: &[u8],
    f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
) -> StorageResult<()> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let mut erased = <dyn erased_serde::Deserializer>::erase(&mut de);
    f(&mut erased).attach_value_bytes(bytes.len())
}

fn text_of(bytes: &[u8]) -> StorageResult<&str> {
    std::str::from_utf8(bytes)
        .map_err(|why| CodecError::Custom(why.to_string()))
        .change_context(StorageError::Codec)
}

/// What a migration pass reads and writes: the page's storage, under one
/// store's keys.
struct OnPage<'a> {
    page: Page,
    keys: &'a Keys,
}

impl MigrationBackendAdapter for OnPage<'_> {
    fn format(&self) -> CodecFormat {
        CodecFormat::Json
    }

    fn get(&self, key: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        Ok(self
            .page
            .get(&self.keys.value(key))?
            .map(String::into_bytes))
    }

    fn set(&mut self, key: &StorePath, value: &[u8]) -> StorageResult<()> {
        self.page.set(&self.keys.value(key), text_of(value)?)
    }

    fn delete(&mut self, key: &StorePath) -> StorageResult<()> {
        self.page.remove(&self.keys.value(key))
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        Ok(self
            .page
            .values_under(self.keys, prefix)?
            .into_iter()
            .map(|(path, text)| (path, text.into_bytes()))
            .collect())
    }

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
        self.page.read(&self.keys.about(META, prefix))
    }

    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()> {
        self.page.write(&self.keys.about(META, prefix), meta)
    }

    fn get_schema_snapshots(&self, prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>> {
        Ok(self
            .page
            .read(&self.keys.about(SCHEMAS, prefix))?
            .unwrap_or_default())
    }

    fn set_schema_snapshots(
        &mut self,
        prefix: &StorePath,
        trees: &[SchemaSnapshot],
    ) -> StorageResult<()> {
        self.page.write(&self.keys.about(SCHEMAS, prefix), trees)
    }

    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>> {
        self.page.read(&self.keys.about(LOG, prefix))
    }

    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()> {
        self.page.write(&self.keys.about(LOG, prefix), log)
    }
}

/// Runs a migration pass over the page's storage, putting back every key the
/// store had before it if the pass fails.
struct PageProvider<'a>(&'a Keys);

impl StorageProvider for PageProvider<'_> {
    fn atomic<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>,
    {
        let mut on = OnPage {
            page: Page::open()?,
            keys: self.0,
        };
        let before = self.0.named(on.page.everything_of(self.0))?;
        let outcome = f(&mut on);
        if let Err(failed) = outcome {
            return self.0.named(Err(match on.page.put_back(self.0, &before) {
                Ok(()) => failed,
                Err(also) => failed.attach(format!(
                    "and what the store held before the pass could not be put back: {also:?}"
                )),
            }));
        }
        outcome
    }
}

impl LocalStorageStore {
    /// The store named `config.path` in this page's `localStorage`, with
    /// `migrations` run over it.
    ///
    /// Only the name, the limits and what to do when the store will not open
    /// apply from `config`: there is no file to reach, nothing to save, and a
    /// page has one thread to read on.
    pub fn open(
        config: &StoreConfig,
        migrations: MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        Page::open().attach("opening the store")?;

        let keys = Keys {
            root: root_of(&config.path),
        };
        let read = Page::open().and_then(|page| page.bookkeeping_reads(&keys));
        if let Err(why) = keys.named(read)
            && !start_fresh(config, &keys, &why)
        {
            return Err(why.attach("opening the store"));
        }

        let store = Self {
            inner: Arc::new(Inner {
                keys,
                subscriptions: RwLock::new(Vec::new()),
                settled: AtomicU64::new(0),
                commits: Arc::new(CommitSignal::default()),
                closed: AtomicBool::new(false),
                budget: Screening::resolve(&config.limits, Backend::LocalStorage),
            }),
        };

        let keys = &store.inner.keys;
        keys.named(format::settle(&store, Backend::LocalStorage))
            .attach("opening the store")?;

        let report = keys
            .named(MigrationEngine::new(&PageProvider(keys)).run(migrations))
            .attach("opening the store")?;

        #[cfg(target_arch = "wasm32")]
        other_pages::listen(&store.inner);

        Ok((store, report))
    }

    fn refuse_if_closed(&self) -> StorageResult<()> {
        self.inner
            .keys
            .named(match self.inner.closed.load(Ordering::Acquire) {
                true => {
                    Err(Report::new(StorageError::Closed).attach("the store lived in the page"))
                }
                false => Ok(()),
            })
    }

    fn page(&self) -> StorageResult<Page> {
        self.refuse_if_closed()?;
        self.inner.keys.named(Page::open())
    }

    fn named<T>(&self, work: impl FnOnce() -> StorageResult<T>) -> StorageResult<T> {
        self.inner.keys.named(work())
    }

    fn encode(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
    ) -> StorageResult<String> {
        let budget = &self.inner.budget;
        budget.check_path(path)?;

        let depth = budget.for_value(path);
        let counted = depth.count(value);
        let mut bytes = Vec::new();
        let mut ser = serde_json::Serializer::new(&mut bytes);

        serde::Serialize::serialize(&counted, &mut ser)
            .map_err(CodecError::from)
            .map_err(|why| match depth.overflowed() {
                true => budget.too_deep(path),
                false => Report::new(why)
                    .change_context(StorageError::Codec)
                    .attach(Key(path.clone())),
            })?;

        budget.settle_the_enum_for_a_reader(&depth, value, path);

        if let Some(refusal) = budget.refused(&depth, path) {
            return Err(refusal);
        }

        String::from_utf8(bytes)
            .map_err(|why| CodecError::Custom(why.to_string()))
            .change_context(StorageError::Codec)
    }
}

impl Inner {
    fn settle(&self) -> u64 {
        self.settled.fetch_add(1, Ordering::AcqRel) + 1
    }
}

impl StoreBackend for LocalStorageStore {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        let page = self.page()?;
        self.named(|| {
            Ok(page
                .get(&self.inner.keys.value(path))?
                .map(String::into_bytes))
        })
    }

    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.set_owned_erased(path.clone(), value, source)
    }

    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        let page = self.page()?;
        let key = self.inner.keys.value(&path);
        let (text, old) = self.named(|| {
            let text = self.encode(&path, value)?;
            let old = page.get(&key)?;
            Ok((text, old))
        })?;

        if old.as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        self.named(|| page.set(&key, &text).attach(Key(path.clone())))?;
        let settled = self.inner.settle();

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path,
                op: StoreOp::Set,
                old: old.map(String::into_bytes),
                new: Some(text.into_bytes()),
                source: source.into(),
                at: settled,
            },
        )
    }

    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool> {
        let Some(bytes) = self.get_raw(path)? else {
            return Ok(false);
        };

        let decoded = decode(&bytes, f).change_context(StorageError::Read);
        self.inner.keys.named(decoded).attach_key(path)?;
        Ok(true)
    }

    fn decode_erased(
        &self,
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        self.named(|| decode(bytes, f))
    }

    fn delete_with_source(&self, path: &StorePath, source: Option<Uuid>) -> StorageResult<()> {
        let page = self.page()?;
        let key = self.inner.keys.value(path);

        let Some(old) = self.named(|| page.get(&key))? else {
            return Ok(());
        };
        self.named(|| page.remove(&key))?;
        let settled = self.inner.settle();

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path: path.clone(),
                op: StoreOp::Delete,
                old: Some(old.into_bytes()),
                new: None,
                source: source.into(),
                at: settled,
            },
        )
    }

    fn delete(&self, path: &StorePath) -> StorageResult<()> {
        self.delete_with_source(path, None)
    }

    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        let page = self.page()?;
        self.named(|| {
            for (path, _) in page.values_under(&self.inner.keys, prefix)? {
                page.remove(&self.inner.keys.value(&path))?;
            }
            Ok(())
        })?;
        let settled = self.inner.settle();

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path: prefix.clone(),
                op: StoreOp::DeletePrefix,
                old: None,
                new: None,
                source: source.into(),
                at: settled,
            },
        )
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        let page = self.page()?;
        self.named(|| {
            Ok(page
                .values_under(&self.inner.keys, prefix)?
                .into_iter()
                .map(|(path, text)| (path, text.into_bytes()))
                .collect())
        })
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        let page = self.page()?;
        let mut found: Vec<StorePath> = self
            .named(|| page.keys())?
            .iter()
            .filter_map(|key| self.inner.keys.path_of(key))
            .filter(|path| path.starts_with(prefix))
            .collect();
        found.sort();
        Ok(found)
    }

    fn files_layout(&self) -> Option<StoreLayout> {
        Some(StoreLayout::PageStorage {
            under: self.inner.keys.root.clone(),
        })
    }

    #[cfg(feature = "test-utils")]
    fn format_record(&self) -> Option<&dyn crate::store::format::TestFormatRecord> {
        Some(self)
    }

    fn save_now(&self) -> StorageResult<()> {
        self.refuse_if_closed()
    }

    fn close(&self) -> StorageResult<()> {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.commits.closed();
        #[cfg(target_arch = "wasm32")]
        other_pages::stop(&self.inner);
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        utils::subscribe(&self.inner.subscriptions, kind, callback)
    }

    fn unsubscribe(&self, id: SubscriptionId) -> bool {
        utils::unsubscribe(&self.inner.subscriptions, id)
    }

    fn flush_prefix(&self, _prefix: &StorePath) -> StorageResult<()> {
        self.refuse_if_closed()
    }

    fn flush_async(&self) -> Commit {
        if self.is_closed() {
            return Commit::gone();
        }

        let signal = &self.inner.commits;
        let commit = Commit::awaiting(signal.clone());
        let flush = signal.begin();
        signal.settle(flush, &Ok(()));
        commit
    }

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        let page = self.page()?;
        self.named(|| {
            Ok(page
                .get(&self.inner.keys.about(SEEDED, namespace))?
                .is_some())
        })
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        let page = self.page()?;
        let key = self.inner.keys.about(SEEDED, namespace);
        self.named(|| match state.is_seeded() {
            true => page.set(&key, "true"),
            false => page.remove(&key),
        })
    }

    fn record_schema(&self, at: &StorePath, schema: &SchemaSnapshot) -> StorageResult<()> {
        let page = self.page()?;
        let key = self.inner.keys.about(SCHEMAS, at);
        self.named(|| {
            let mut trees: Vec<SchemaSnapshot> = page.read(&key)?.unwrap_or_default();
            crate::store::moved::record_into(&mut trees, schema);
            page.write(&key, &trees)
        })
    }
}

impl InspectorBackend for LocalStorageStore {
    fn format(&self) -> CodecFormat {
        CodecFormat::Json
    }

    fn scan_all(&self) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.scan_prefix(&StorePath::root())
            .attach("inspecting every key in the store")
    }

    fn get_schema_snapshots(&self) -> StorageResult<Vec<(String, SchemaSnapshot)>> {
        let page = self.page()?;

        self.named(|| {
            let mut found = Vec::new();
            for key in page.keys()? {
                let prefix = match self.inner.keys.read(&key) {
                    Some((kind, prefix)) if kind == SCHEMAS => prefix,
                    _ => continue,
                };
                let trees: Vec<SchemaSnapshot> = page.read(&key)?.unwrap_or_default();
                found.extend(trees.into_iter().map(|tree| (prefix.to_string(), tree)));
            }
            found.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok(found)
        })
    }

    fn set_raw(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        let path = StorePath::parse_joined(key)
            .change_context(StorageError::Write)
            .attach_raw_key(key)?;
        let page = self.page()?;

        self.named(|| {
            let text = text_of(value)?;
            page.set(&self.inner.keys.value(&path), text)
                .attach(Key(path.clone()))
        })
    }
}

impl format::FormatRecord for LocalStorageStore {
    fn format_facts(&self) -> StorageResult<Option<StorageFactSet>> {
        self.named(|| Page::open()?.read(&self.inner.keys.format()))
    }

    fn set_format_facts(&self, facts: &StorageFactSet) -> StorageResult<()> {
        self.named(|| Page::open()?.write(&self.inner.keys.format(), facts))
    }
}

/// Changes other pages of the same site make to a store's keys, which the
/// browser announces to every page but the one that made them.
#[cfg(target_arch = "wasm32")]
mod other_pages {
    use super::*;
    use amethystate_core::change::Source;
    use std::cell::RefCell;
    use std::sync::Weak;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    type Listener = Closure<dyn FnMut(web_sys::StorageEvent)>;

    thread_local! {
        static LISTENING: RefCell<Vec<(usize, Listener)>> = const { RefCell::new(Vec::new()) };
    }

    fn id_of(inner: &Arc<Inner>) -> usize {
        Arc::as_ptr(inner) as usize
    }

    pub(super) fn listen(inner: &Arc<Inner>) {
        let Some(window) = web_sys::window() else {
            return;
        };

        let weak: Weak<Inner> = Arc::downgrade(inner);
        let listener: Listener = Closure::new(move |event: web_sys::StorageEvent| {
            if let Some(inner) = weak.upgrade() {
                inner.heard(&event);
            }
        });

        if window
            .add_event_listener_with_callback("storage", listener.as_ref().unchecked_ref())
            .is_ok()
        {
            LISTENING.with(|all| all.borrow_mut().push((id_of(inner), listener)));
        }
    }

    pub(super) fn stop(inner: &Arc<Inner>) {
        let id = id_of(inner);
        let gone: Vec<Listener> = LISTENING.with(|all| {
            let mut all = all.borrow_mut();
            let (gone, kept) = all.drain(..).partition(|(of, _)| *of == id);
            *all = kept;
            gone.into_iter().map(|(_, listener)| listener).collect()
        });

        if let Some(window) = web_sys::window() {
            for listener in &gone {
                let _ = window.remove_event_listener_with_callback(
                    "storage",
                    listener.as_ref().unchecked_ref(),
                );
            }
        }
    }

    impl Inner {
        fn heard(&self, event: &web_sys::StorageEvent) {
            if self.closed.load(Ordering::Acquire) {
                return;
            }

            let ours = web_sys::window().and_then(|window| window.local_storage().ok().flatten());
            if event.storage_area() != ours {
                return;
            }

            let announced = match event.key() {
                None => StoreEvent {
                    path: StorePath::root(),
                    op: StoreOp::DeletePrefix,
                    old: None,
                    new: None,
                    source: Source::AnotherPage,
                    at: self.settle(),
                },
                Some(key) => {
                    let Some(path) = self.keys.path_of(&key) else {
                        return;
                    };
                    let new = event.new_value().map(String::into_bytes);
                    StoreEvent {
                        path,
                        op: match new {
                            Some(_) => StoreOp::Set,
                            None => StoreOp::Delete,
                        },
                        old: event.old_value().map(String::into_bytes),
                        new,
                        source: Source::AnotherPage,
                        at: self.settle(),
                    }
                }
            };

            if let Err(why) = utils::emit_events(&self.subscriptions, announced) {
                tracing::warn!("a change another page made could not be announced: {why:?}");
            }
        }
    }
}
