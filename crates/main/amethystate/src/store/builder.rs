use crate::migration::builder::MigrationBuilder;
use crate::store::config::{Disk, FileWritePolicy, StoreConfig, WriteLimits};
use crate::store::facts::Facts;
use crate::store::{StorageError, StorageResult};
use crate::{MigrationReport, Store};
use error_stack::{Report, ResultExt};
use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Which engine backs a store.
///
/// A variant exists for each backend feature that is enabled. Pass one to
/// [`StoreBuilder::backend`]; without that, [`default_backend`] picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    #[cfg(feature = "redb")]
    Redb,
    #[cfg(feature = "json")]
    Json,
    #[cfg(feature = "toml")]
    Toml,
    #[cfg(feature = "ron")]
    Ron,
    #[cfg(feature = "sqlite")]
    Sqlite,
}

impl Backend {
    pub const fn extension(self) -> &'static str {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => "redb",
            #[cfg(feature = "json")]
            Backend::Json => "json",
            #[cfg(feature = "toml")]
            Backend::Toml => "toml",
            #[cfg(feature = "ron")]
            Backend::Ron => "ron",
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => "db",
        }
    }

    /// How deeply this engine's codec will read, counting the path and the
    /// value together. A fact about the codec, enforced whatever the store is
    /// configured to allow.
    ///
    /// Every codec here reads less deeply than it writes, so a value past this
    /// is taken without complaint and cannot be read back - which on the text
    /// engines means the whole file, since the document is parsed as one. Each
    /// number comes from `tests/probe_*.rs`, which walks its engine to the
    /// boundary.
    ///
    /// `redb` has no limit of its own: `rmp_serde` recurses until the stack
    /// ends, around three thousand levels, and the process dies there - on
    /// every later start, because the value is already committed. Its number is
    /// imposed for that reason, far above any data anyone means to store and
    /// far below where the stack gives out.
    ///
    /// sqlite takes json's number. It stores its values as JSON, encoded by
    /// `sonic_rs`, so it belongs beside json rather than out at the 254 a walk
    /// to its give-out first suggested: a measured boundary is where a library
    /// happened to stop, not a promise about its next release, and two engines
    /// carrying the same format answering 127 levels apart is worse than
    /// either answer.
    pub const fn depth_ceiling(self) -> usize {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => 512,
            #[cfg(feature = "json")]
            Backend::Json => 127,
            #[cfg(feature = "toml")]
            Backend::Toml => 80,
            #[cfg(feature = "ron")]
            Backend::Ron => 64,
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => 127,
        }
    }

    /// Whether committing one write on this engine commits every other write
    /// waiting beside it.
    ///
    /// A document engine keeps the store in one file and rewrites the whole of
    /// it to save any of it, so `flush_prefix` there ignores its prefix and
    /// calls `save_now`: asking for one value to be durable makes every
    /// buffered value durable with it. redb and sqlite commit the write that
    /// was asked for and leave the rest in the buffer.
    ///
    /// The document engines are not going to be taught otherwise. Splitting an
    /// in-memory document into per-key writes buys nothing a caller asked for -
    /// the file is rewritten either way - and would exist only to make the two
    /// kinds of engine behave alike. So this is the answer rather than a gap,
    /// and the tests read it from here instead of each naming a number.
    ///
    /// What it decides is what survives a crash, which is the one place the
    /// difference shows from outside: `tests/durability_crash.rs` kills a
    /// process with one durable write and one plain one pending, and asks
    /// this.
    pub const fn a_commit_covers_the_whole_store(self) -> bool {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => false,
            #[cfg(feature = "json")]
            Backend::Json => true,
            #[cfg(feature = "toml")]
            Backend::Toml => true,
            #[cfg(feature = "ron")]
            Backend::Ron => true,
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => false,
        }
    }

    /// Whether this engine can carry an integer that does not fit in an `i64`.
    ///
    /// TOML has one integer type and it is signed and 64 bits wide, so
    /// `u64::MAX` has nowhere to go. Its own codec refuses the write, which is
    /// loud enough while toml is the engine running - but a store on any other
    /// engine that named toml in `portable_across` would otherwise take the
    /// value and break the promise quietly.
    pub const fn holds_an_integer_past_i64(self) -> bool {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => true,
            #[cfg(feature = "json")]
            Backend::Json => true,
            #[cfg(feature = "toml")]
            Backend::Toml => false,
            #[cfg(feature = "ron")]
            Backend::Ron => true,
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => true,
        }
    }

    /// Whether this engine can write a `NaN` or an infinity and read it back.
    ///
    /// JSON has no spelling for either, and `serde_json` follows
    /// `JSON.stringify` in writing `null` - which then fails to decode as a
    /// float. sqlite answers the same way for the same reason: it encodes its
    /// values with `sonic_rs`. So the split follows the codec rather than the
    /// file, which is why the two are not the two text engines anyone would
    /// guess: msgpack, TOML and RON all carry the value.
    pub const fn holds_non_finite_floats(self) -> bool {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => true,
            #[cfg(feature = "json")]
            Backend::Json => false,
            #[cfg(feature = "toml")]
            Backend::Toml => true,
            #[cfg(feature = "ron")]
            Backend::Ron => true,
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => false,
        }
    }

    /// Whether this engine can write an enum and read it back.
    ///
    /// ron cannot, and the reason is its document type rather than its syntax:
    /// `ron::value::Value` holds nine shapes and none of them is a variant, so
    /// a value rendered as `On(3)` and parsed back into a `Value` arrives as a
    /// sequence with the name gone. Upstream lists it - "enums not supported"
    /// is an open item of [ron-rs/ron#122] - and ron's own deserializer says
    /// the same of itself.
    ///
    /// Avoiding the round trip would take a `to_value`, which ron does not
    /// have either: [ron-rs/ron#140], open since 2018 across four releases
    /// that were each meant to carry it.
    ///
    /// [ron-rs/ron#122]: https://github.com/ron-rs/ron/issues/122
    /// [ron-rs/ron#140]: https://github.com/ron-rs/ron/issues/140
    pub const fn holds_enums(self) -> bool {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => true,
            #[cfg(feature = "json")]
            Backend::Json => true,
            #[cfg(feature = "toml")]
            Backend::Toml => true,
            #[cfg(feature = "ron")]
            Backend::Ron => false,
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => true,
        }
    }

    /// Whether this engine tells `Some(None)` apart from `None`.
    ///
    /// Only ron does, and it is the one that writes an `Option` in words:
    /// `Some(None)` is in the document as itself. Everywhere else the outer
    /// `Some` has nothing of its own to write, so both values reach the file as
    /// one null - `c0` under msgpack, `null` under either JSON - and both read
    /// back as `None`. toml has no null at all and refuses the pair outright.
    ///
    /// It is serde's own representation rather than anything this crate does,
    /// so no engine can be taught otherwise; the write is refused instead.
    pub const fn keeps_a_nested_option(self) -> bool {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => false,
            #[cfg(feature = "json")]
            Backend::Json => false,
            #[cfg(feature = "toml")]
            Backend::Toml => false,
            #[cfg(feature = "ron")]
            Backend::Ron => true,
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => false,
        }
    }

    /// Opens a store on this engine directly, skipping the builder.
    ///
    /// [`StoreBuilder`] is the ordinary route - it also collects migrations
    /// and settings this does not.
    pub fn open_public(
        self,
        config: StoreConfig,
        mset: crate::migration::set::MigrationSet,
    ) -> StorageResult<(Store, MigrationReport)> {
        match self {
            #[cfg(feature = "redb")]
            Backend::Redb => {
                let (s, r) = crate::store::backend::redb::RedbStore::open(config, mset)?;
                Ok((Store::from_arc(Arc::new(s)), r))
            }
            #[cfg(feature = "json")]
            Backend::Json => {
                let (s, r) = crate::store::backend::text::JsonStore::open(config, mset)?;
                Ok((Store::from_arc(Arc::new(s)), r))
            }
            #[cfg(feature = "toml")]
            Backend::Toml => {
                let (s, r) = crate::store::backend::text::TomlStore::open(config, mset)?;
                Ok((Store::from_arc(Arc::new(s)), r))
            }
            #[cfg(feature = "ron")]
            Backend::Ron => {
                let (s, r) = crate::store::backend::text::RonStore::open(config, mset)?;
                Ok((Store::from_arc(Arc::new(s)), r))
            }
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => {
                let (s, r) = crate::store::backend::sqlite::SqliteStore::open(config, mset)?;
                Ok((Store::from_arc(Arc::new(s)), r))
            }
        }
    }
}

/// Expands a priority list into the `not(...)` cascade that picking the first
/// enabled feature otherwise requires by hand.
macro_rules! first_enabled_backend {
    ($feat:literal => $variant:expr $(, $rest_feat:literal => $rest_variant:expr)* $(,)?) => {
        {
            #[cfg(feature = $feat)]
            { $variant }
            #[cfg(not(feature = $feat))]
            { first_enabled_backend!($($rest_feat => $rest_variant),*) }
        }
    };
    () => {
        compile_error!(
            "amethystate needs at least one storage backend feature: redb, sqlite, json, toml or ron"
        )
    };
}

/// The engine used when the caller does not name one.
///
/// The first of redb, sqlite, json, toml, ron that is enabled. Naming the
/// engine with [`StoreBuilder::backend`] is worth doing wherever it matters
/// which one runs - the on-disk format differs, and so does what a durable
/// write commits.
pub const fn default_backend() -> Backend {
    first_enabled_backend! {
        "redb"   => Backend::Redb,
        "sqlite" => Backend::Sqlite,
        "json"   => Backend::Json,
        "toml"   => Backend::Toml,
        "ron"    => Backend::Ron,
    }
}

/// How a [`Store`] is opened: where its file is, which engine reads it, and
/// what the store does while it runs.
///
/// Two ways in. [`StoreBuilder::new`] takes a path the caller already has;
/// [`StoreBuilder::located`] works one out from what the machine says, and is
/// where the platform's configuration directory and a portable install beside
/// the executable live.
///
/// ```no_run
/// use amethystate::store::builder::StoreBuilder;
///
/// let store = StoreBuilder::new("./settings").build()?;
/// # Ok::<(), error_stack::Report<amethystate::store::StorageError>>(())
/// ```
///
/// Everything past that is optional and reads in one chain. Naming the engine
/// matters wherever it matters which format is on disk, because
/// [`default_backend`] picks the first one the build has:
///
/// ```no_run
/// use amethystate::store::builder::{Layout, StoreBuilder, default_backend};
/// use amethystate::store::config::WriteAttempts;
/// use std::time::Duration;
///
/// // `Backend::Json` and the rest exist only where their feature is on, so an
/// // example that has to compile under every one of them names none.
/// let store = StoreBuilder::located(|at| at.app_under(Layout::Native, "my-app", "settings"))?
///     .backend(default_backend())
///     .disk(|d| d.debounce(Duration::from_millis(500)))
///     .file_write(|w| w.replacing(WriteAttempts::times(20).apart(Duration::from_millis(250))))
///     .build()?;
/// # Ok::<(), error_stack::Report<amethystate::store::StorageError>>(())
/// ```
///
/// A store that migrations run against is built the same way, with
/// [`StoreBuilder::migrations`] and [`StoreBuilder::build_with_migration`] when
/// what they did is wanted back.
pub struct StoreBuilder {
    backend: Backend,
    config: StoreConfig,
    migration_builder: MigrationBuilder,
    check_context: crate::store::CheckContext,
    /// Whether the extension on the path was spelled by the caller.
    ///
    /// An extension this crate chose belongs to whichever engine is going to
    /// run, and so has to follow [`StoreBuilder::backend`]; one the caller
    /// wrote is theirs, and naming an engine does not overrule it.
    caller_named_extension: bool,
}

/// Which convention decides where an application's files belong.
///
/// A store is found under the layout it was written under, so an application
/// that has shipped should name the one it means. [`Location::app`] takes
/// [`Layout::App`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Layout {
    /// The convention a command-line application follows: the XDG configuration
    /// directory on Linux and macOS, `AppData\Roaming\<app>\config` on Windows.
    /// What [`Location::app`] uses.
    App,

    /// The convention the rest of the platform's software follows, which is
    /// what a desktop application usually wants: `Library/Preferences/rs.<app>`
    /// on macOS, and the same place as [`Layout::App`] on Linux and Windows.
    Native,

    /// The layout the `directories` crate produces: the XDG configuration
    /// directory on Linux, `AppData\Roaming\<app>\config` on Windows, and
    /// `Library/Application Support/rs.<app>` on macOS.
    ///
    /// The one to name for an application whose files that crate already
    /// placed, since where it put them is what the store has to match. On
    /// Windows it lands where [`Layout::App`] does; on Linux it spells the
    /// application name as given, where [`Layout::App`] lowercases it and turns
    /// spaces into hyphens.
    ProjectDirs,
}

impl Layout {
    /// What [`Location::app`] picks when the caller does not say: [`Layout::App`].
    pub const fn default_for_build() -> Self {
        Self::App
    }
}

/// The places [`StoreBuilder::located`] knows how to find, one method each.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Location;

impl Location {
    /// The configuration directory this platform keeps for `app_name`.
    ///
    /// That is [`Layout::App`]; name a convention with [`Location::app_under`]
    /// where the store must not move.
    pub fn app(
        self,
        app_name: impl AsRef<str>,
        config_name: impl AsRef<str>,
    ) -> StorageResult<PathBuf> {
        self.app_under(Layout::default_for_build(), app_name, config_name)
    }

    /// The same, under the layout named.
    pub fn app_under(
        self,
        layout: Layout,
        app_name: impl AsRef<str>,
        config_name: impl AsRef<str>,
    ) -> StorageResult<PathBuf> {
        let app_name = app_name.as_ref();
        let args = || etcetera::AppStrategyArgs {
            top_level_domain: "rs".to_string(),
            author: String::new(),
            app_name: app_name.to_string(),
        };

        let path = match layout {
            Layout::App => {
                use etcetera::{AppStrategy, app_strategy::choose_app_strategy};

                choose_app_strategy(args())
                    .change_context(StorageError::Open)
                    .attach("this system has no home directory to put an application's own under")?
                    .config_dir()
            }
            Layout::Native => {
                use etcetera::{AppStrategy, app_strategy::choose_native_strategy};

                choose_native_strategy(args())
                    .change_context(StorageError::Open)
                    .attach("this system has no home directory to put an application's own under")?
                    .config_dir()
            }
            Layout::ProjectDirs => {
                use directories::ProjectDirs;

                ProjectDirs::from("rs", "", app_name)
                    .ok_or_else(|| {
                        Report::new(StorageError::Open)
                            .attach("this system has no configuration directory for an application")
                    })?
                    .config_dir()
                    .to_path_buf()
            }
        }
        .join(config_name.as_ref());

        ensure_parent(&path).attach_with(|| format!("application: {app_name}"))?;
        Ok(path)
    }

    /// Beside the running executable, for an installation that is a folder
    /// somebody unpacked and can move.
    ///
    /// This wants a directory the person running the program can write to.
    /// `Program Files` and `/usr/bin` are not, and on macOS the executable
    /// lives inside the bundle, so a file beside it is inside a signed
    /// directory. Any of those shows up as a failure to open the store, at
    /// startup, before the first write.
    pub fn beside_the_executable(self, file_name: impl AsRef<Path>) -> StorageResult<PathBuf> {
        let exe = std::env::current_exe()
            .change_context(StorageError::Open)
            .attach("the running executable cannot be located, so nothing is beside it")?;

        let dir = exe.parent().ok_or_else(|| {
            Report::new(StorageError::Open)
                .attach(format!("executable: {}", exe.display()))
                .attach("the running executable is not in a directory")
        })?;

        let path = dir.join(file_name);
        ensure_parent(&path).attach_with(|| format!("executable: {}", exe.display()))?;
        Ok(path)
    }
}

fn ensure_parent(path: &Path) -> StorageResult<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    std::fs::create_dir_all(parent)
        .change_context(StorageError::Open)
        .attach_with(|| format!("directory: {}", parent.display()))
        .attach_store_file(path)
}

impl StoreBuilder {
    /// A store at an explicit path.
    ///
    /// An extension that is given is kept, whatever engine ends up running -
    /// the path is the caller's. Without one the engine's own extension is
    /// used, and it follows [`StoreBuilder::backend`] if that names another
    /// engine later. [`StoreBuilder::located`] is the variant that picks a
    /// location as well.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let mut path: PathBuf = path.into();
        let caller_named_extension = path.extension().is_some();
        if !caller_named_extension {
            path.set_extension(default_backend().extension());
        }
        Self {
            backend: default_backend(),
            config: StoreConfig::new(path),
            migration_builder: MigrationBuilder::default(),
            check_context: crate::store::CheckContext::default(),
            caller_named_extension,
        }
    }

    /// A store where the machine says it goes.
    ///
    /// The questions about the machine - which directory this platform keeps
    /// for an application, whether this install is a folder somebody unpacked -
    /// are all answered behind here, in one place. [`StoreBuilder::new`] is the
    /// way in when the path is already known.
    ///
    /// The closure is handed a [`Location`] and returns the path it picked.
    /// Every method on it makes sure the directory above the file exists, so
    /// what comes back is somewhere a store can actually be opened, and reports
    /// what went wrong when it is not.
    ///
    /// ```no_run
    /// # use amethystate::store::builder::{Layout, StoreBuilder};
    /// // The configuration directory this platform keeps for an application.
    /// let store = StoreBuilder::located(|at| at.app("my-app", "settings"))?.build()?;
    ///
    /// // The same, under a named convention - worth spelling once an
    /// // application has shipped.
    /// let store = StoreBuilder::located(|at| {
    ///     at.app_under(Layout::Native, "my-app", "settings")
    /// })?
    /// .build()?;
    ///
    /// // Beside the running executable, for an install that can be moved.
    /// let store = StoreBuilder::located(|at| at.beside_the_executable("settings"))?.build()?;
    /// # Ok::<(), error_stack::Report<amethystate::store::StorageError>>(())
    /// ```
    ///
    /// The file name carries no extension in any of those, so the engine that
    /// runs names it - see [`StoreBuilder::backend`] for what that means when
    /// the engine is chosen afterwards.
    pub fn located(pick: impl FnOnce(Location) -> StorageResult<PathBuf>) -> StorageResult<Self> {
        Ok(Self::new(pick(Location)?))
    }

    /// How long a write waits in the buffer before it is flushed.
    ///
    /// When this store touches the file, and what it does when the file will
    /// not be touched.
    ///
    /// The closure is handed the defaults and gives back what it changed, so
    /// nothing has to be restated and nothing can be silently zeroed by
    /// forgetting to. See [`Disk`] for the knobs.
    ///
    /// ```no_run
    /// # use amethystate::store::builder::StoreBuilder;
    /// # use std::time::Duration;
    /// let store = StoreBuilder::new("settings.json")
    ///     .disk(|d| d.debounce(Duration::from_millis(200)))
    ///     .build()?;
    /// # Ok::<(), error_stack::Report<amethystate::store::StorageError>>(())
    /// ```
    pub fn disk(mut self, configure: impl FnOnce(Disk) -> Disk) -> Self {
        let settled = configure(Disk {
            save_debounce: self.config.save_debounce,
            watch_debounce: self.config.watch_debounce,
            retry_policy: self.config.retry_policy.clone(),
            on_persist_failure: self.config.on_persist_failure.clone(),
        });

        self.config.save_debounce = settled.save_debounce;
        self.config.watch_debounce = settled.watch_debounce;
        self.config.retry_policy = settled.retry_policy;
        self.config.on_persist_failure = settled.on_persist_failure;
        self
    }

    /// How hard one write to one file fights before it reports a failure.
    ///
    /// This sits inside a single flush attempt, below [`Disk::retry_every`]:
    /// only
    /// once it has run out does a flush count as having failed at all. It
    /// applies to the text engines, which replace a file to write it; `redb`
    /// and `sqlite` hold their own handle.
    ///
    /// ```no_run
    /// # use amethystate::store::builder::StoreBuilder;
    /// # use amethystate::store::config::WriteAttempts;
    /// # use std::time::Duration;
    /// let store = StoreBuilder::new("settings.json")
    ///     .file_write(|w| {
    ///         w.replacing(WriteAttempts::times(20).apart(Duration::from_millis(250)))
    ///     })
    ///     .build()?;
    /// # Ok::<(), error_stack::Report<amethystate::store::StorageError>>(())
    /// ```
    pub fn file_write(
        mut self,
        configure: impl FnOnce(FileWritePolicy) -> FileWritePolicy,
    ) -> Self {
        self.config.file_write = configure(self.config.file_write);
        self
    }

    /// What this store refuses to hold, as against what its codec cannot.
    ///
    /// The codec's own ceiling is enforced whatever is set here - a value it
    /// cannot read back is refused always, because taking it produces a file
    /// that will not open. What this adds is the store's own: a cap on how deep
    /// a path may go, and a claim that the contents stay readable on engines
    /// other than the one running.
    ///
    /// ```no_run
    /// # use amethystate::store::builder::{Backend, StoreBuilder, default_backend};
    /// let store = StoreBuilder::new("./settings")
    ///     .limits(|l| l.key_depth(8).portable_across([default_backend()]))
    ///     .build()?;
    /// # Ok::<(), error_stack::Report<amethystate::store::StorageError>>(())
    /// ```
    ///
    /// The claim names the engines it means, so it stays what the application
    /// asked for when this crate gains another one, and so it can be as narrow
    /// as the requirement really is.
    pub fn limits(mut self, configure: impl FnOnce(WriteLimits) -> WriteLimits) -> Self {
        self.config.limits = configure(std::mem::take(&mut self.config.limits));
        self
    }

    /// Declares migration steps to run when the store opens.
    ///
    /// Steps written with `#[migrate]` are collected automatically by
    /// [`StoreBuilder::build_with_migration`]; this is for the ones built by
    /// hand.
    pub fn migrations(mut self, configure: impl FnOnce(&mut MigrationBuilder)) -> Self {
        configure(&mut self.migration_builder);
        self
    }

    /// Hands a value to every migration step that runs when this store opens.
    ///
    /// A step written with `#[migrate]` is collected at link time as a bare
    /// `fn(&mut MigrationContext)`, so it captures nothing: anything it needs
    /// from the application - a lookup table, a client, the settings it is
    /// porting away from - has no way in except a global. This is that way in.
    ///
    /// One value per type; the step asks for it back with
    /// [`MigrationContext::provided`] or [`MigrationContext::require`].
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// struct LegacyDefaults {
    ///     port: u16,
    /// }
    ///
    /// let store = StoreBuilder::new(&*path)
    ///     .provide(LegacyDefaults { port: 8080 })
    ///     .build()
    ///     .unwrap();
    /// # let _ = store;
    /// ```
    ///
    /// [`MigrationContext::provided`]: crate::MigrationContext::provided
    /// [`MigrationContext::require`]: crate::MigrationContext::require
    pub fn provide<T: Any>(mut self, value: T) -> Self {
        self.migration_builder.provide(value);
        self
    }

    /// Hands a value to every check this store's declared structs run.
    ///
    /// A check written with `#[amestate(check = ..)]` is a bare `fn` and
    /// captures nothing, so the world it has to judge a value against - which
    /// monitors exist, which themes are installed, what this machine allows -
    /// arrives here. One value per type; the check asks for it back with
    /// [`CheckContext::get`] or [`CheckContext::require`].
    ///
    /// The `Send + Sync` bound is what separates this from
    /// [`StoreBuilder::provide`]. A migration step runs once, inside
    /// [`build`](StoreBuilder::build), on the thread that called it. A check
    /// runs every time a value arrives, including from the thread that watches
    /// the file, so what it reads has to be readable from there.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc_context");
    /// struct Monitors {
    ///     count: usize,
    /// }
    ///
    /// let store = StoreBuilder::new(&*path)
    ///     .context(Monitors { count: 2 })
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(store.context().get::<Monitors>().unwrap().count, 2);
    /// ```
    ///
    /// [`CheckContext::get`]: crate::store::CheckContext::get
    /// [`CheckContext::require`]: crate::store::CheckContext::require
    pub fn context<T: Any + Send + Sync>(mut self, value: T) -> Self {
        self.check_context.insert(value);
        self
    }

    /// Lets reading a large collection back use more than one core.
    ///
    /// Parsing every stored key and decoding every value is around four
    /// hundred milliseconds of a million-entry open, and dividing them takes
    /// that to about eighty. Off by default: this is a thread pool inside a
    /// state library, and an application that already has one should say
    /// whether it wants a second. While it is off nothing is spawned - the
    /// pool is built on first use.
    ///
    /// Small collections are unaffected either way: below roughly a thousand
    /// entries the handing out costs more than the work, and the split does
    /// not happen.
    pub fn parallel_reads(mut self, yes: bool) -> Self {
        self.config.parallel_reads = yes;
        self
    }

    /// Picks the engine explicitly. Without this the store uses
    /// [`default_backend`].
    ///
    /// An extension this crate chose moves with the engine: a path left
    /// without one by [`StoreBuilder::new`] or [`StoreBuilder::located`] is
    /// named for whichever engine actually runs, so a store asked for as
    /// `json` is not opened on a file called `.redb`. An extension the caller
    /// spelled stays as it is.
    pub fn backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        if !self.caller_named_extension {
            self.config.path.set_extension(backend.extension());
        }
        self
    }

    /// Opens the store, running the migrations declared by hand and no others.
    ///
    /// `#[migrate]` steps are collected by
    /// [`StoreBuilder::build_with_migration`], which is the only path that
    /// finds them; a store opened here never sees them and says nothing about
    /// it.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// let store = StoreBuilder::new(&*path).build().unwrap();
    /// store.kv().set("a", &1u8).unwrap();
    /// assert_eq!(store.kv().get::<u8>("a").unwrap(), Some(1));
    /// ```
    pub fn build(self) -> StorageResult<Store> {
        let context = Arc::new(self.check_context);
        let migration_set = self.migration_builder.into_set();
        let (store, _) = self.backend.open_public(self.config, migration_set)?;

        Ok(store.with_context(context))
    }

    /// Opens the store and returns what the migration pass did.
    ///
    /// This is also the path that collects `#[migrate]` steps, so a store
    /// opened with [`StoreBuilder::build`] runs only the migrations declared
    /// by hand.
    pub fn build_with_migration(mut self) -> StorageResult<(Store, MigrationReport)> {
        self.migration_builder.collect_codegen();
        let context = Arc::new(self.check_context);
        let migration_set = self.migration_builder.into_set();
        let (store, report) = self.backend.open_public(self.config, migration_set)?;
        report.log_to_tracing();
        Ok((store.with_context(context), report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "redb", feature = "json"))]
    #[test]
    fn a_defaulted_extension_follows_the_engine_that_is_named() {
        let builder = StoreBuilder::new("app/settings").backend(Backend::Json);

        assert_eq!(
            builder.config.path.extension().and_then(|e| e.to_str()),
            Some("json"),
            "the engine was named after the path was built, and the path did not follow"
        );
    }

    #[cfg(all(feature = "redb", feature = "json", feature = "toml"))]
    #[test]
    fn the_last_engine_named_is_the_one_the_path_follows() {
        let builder = StoreBuilder::new("app/settings")
            .backend(Backend::Json)
            .backend(Backend::Toml);

        assert_eq!(
            builder.config.path.extension().and_then(|e| e.to_str()),
            Some("toml")
        );
    }

    #[cfg(all(feature = "redb", feature = "json"))]
    #[test]
    fn an_extension_the_caller_wrote_is_left_alone() {
        let builder = StoreBuilder::new("app/settings.conf").backend(Backend::Json);

        assert_eq!(
            builder.config.path.extension().and_then(|e| e.to_str()),
            Some("conf")
        );
    }

    #[test]
    fn an_unnamed_engine_keeps_the_default_extension() {
        let builder = StoreBuilder::new("app/settings");

        assert_eq!(
            builder.config.path.extension().and_then(|e| e.to_str()),
            Some(default_backend().extension())
        );
    }
}
