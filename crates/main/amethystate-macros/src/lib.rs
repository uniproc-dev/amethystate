use proc_macro::TokenStream;

mod amethystate;
mod migrate;
mod ts_mapping;

/// Generates a persistent state wrapper for a struct.
///
/// This macro creates structures that manage persistence, reactive subscribers,
/// and migrations. Depending on the selected `mode`, it generates either reactive
/// `Field<T>` accessors or a flat persistent-only model.
///
/// # Struct Attributes (`#[amethystate(...)]`)
///
/// * `#[amethystate(prefix = "path", version = 1, mode = "reactive")]` - Defines a **Root** struct.
///   * `as_root` (optional flag): If specified, fields are written directly to the store root without
///     a namespace. It cannot be given beside `prefix`.
///   * `prefix` (String): Sets the top-level namespace path in the store.
///     Generates `pub fn new() -> Result<Self, OpenStruct>`, which opens on the
///     global store, and `pub fn new_with(store: &Store) -> Result<Self, OpenStruct>`
///     for a store the caller holds.
///   * `version` (optional u32): Schema version for migrations (defaults to 0).
///   * `mode` (optional String): Controls the generated code paradigm. One of:
///     * `"reactive"` (default): Generates fine-grained reactive `Field<T>` accessors.
///     * `"persistent"`: Generates a flat struct with plain-type fields and synchronous `.save()` / `.save_lazy()` methods.
///     * `"both"`: Generates both reactive accessors on `#name` and a separate `#name_Persistent` flat struct.
///   * `check` (optional path): A `fn(&Data, &CheckContext) -> Result<(), Invalid>`
///     run over the whole struct as it is built.
///   * `on_unreadable` / `on_delete` / `unreadable_entries` (optional paths):
///     What every field of this struct falls back to when it says nothing
///     itself - see `store::OnUnreadable`, `store::OnDelete` and
///     `store::UnreadableEntries`. The third is about a map's entries, and a
///     field that is not a map ignores it.
/// * `#[amethystate]` - Defines a **Nested** struct.
///   * Used as a component within other structures.
///   * Generates `pub fn new(store: &Store, namespace: impl IntoStorePath) -> Result<Self, OpenStruct>`.
///
/// # How a storage path is built
///
/// A value's path is the struct's levels followed by the field's. Both sides
/// are written as a dotted string and taken apart at the dots, so `prefix =
/// "sys.db"` is two levels rather than one name holding a dot. The names go in
/// as written - nothing is derived or mangled - and one that holds the
/// separator or a backslash is escaped when the path is written out as a key.
///
/// | Declaration | Path |
/// | :--- | :--- |
/// | `#[amethystate(prefix = "net")]`, field `port` | `net.port` |
/// | the same, with `#[amestate(path = "listen_port")]` | `net.listen_port` |
/// | nested struct at field `db` inside prefix `sys`, its field `host` | `sys.db.host` |
/// | the same, with `#[amestate(nested, flatten)]` on `db` | `sys.host` |
/// | `#[amethystate(as_root)]`, field `port` | `port` |
///
/// Where a field sits is this macro's to say, so it is said with `amestate`.
/// `#[serde(rename)]` on a declared field is refused rather than read: serde
/// decides how a value is written, and a struct with a prefix does not
/// serialise in one pass for it to have an opinion about.
///
/// `as_root` gives the struct no levels of its own, so a field's key is the
/// whole path.
///
/// Every level has to have a name, so `prefix = ""`, `prefix = "."`, `prefix =
/// "a..b"` and `prefix = "a."` are refused where they are written. Dropping the
/// nameless level instead would turn a mistyped prefix into a struct scoped to
/// the root, which is a thing a struct is allowed to be: write `as_root` when
/// that is what was meant.
///
/// The levels are taken apart here, at expansion, and the struct carries them
/// as `StateScope::PATH`; nothing splits a string at startup. A field reports
/// where it ended up through `Field::path`.
///
/// # Field Attributes (`#[amestate(...)]`)
///
/// | Option | Form | Description |
/// | :--- | :--- | :--- |
/// | `default` | `= Expr` | Initial value if not present in store. Falls back to `Default::default()`. |
/// | `path` | `= String` | Where the field sits, instead of its own name. A dot in it is a level. |
/// | `check` | `= path` | A `fn(&T, &CheckContext) -> Result<(), Invalid>` every value coming in from the store has to pass. |
/// | `on_unreadable` | `= path` | What this field does about a stored value it will not accept - see `store::OnUnreadable`. |
/// | `on_delete` | `= path` | What this field does when its key is deleted under it - see `store::OnDelete`. |
/// | `unreadable_entries` | `= path` | On a `ReactiveMap`: what it does with an entry it cannot read - see `store::UnreadableEntries`. |
/// | `nested` | flag | Marks field as another `#[amethystate]` struct. |
/// | `flatten` | flag | On a `nested` field: its fields sit at this level, and it takes no segment of its own. |
/// | `volatile` | flag | In-memory only. Never saved to or loaded from disk. |
///
/// `nested`, `flatten` and `volatile` are bare flags: they are written on their
/// own, with no `= true`.
///
/// # Examples
///
/// ### Reactive Mode (Default)
/// ```rust,ignore
/// #[amethystate(prefix = "settings")]
/// pub struct AppSettings {
///     #[amestate(default = "localhost".to_string())]
///     pub host: String,
///
///     #[amestate(default = false, volatile)]
///     pub debug_mode: bool,
/// }
///
/// // Usage:
/// // let settings = AppSettings::new_with(&store)?;
/// // let _sub = settings.host().subscribe(|val| println!("Host: {val}"));
/// // settings.host().set("10.0.0.1".to_string())?;
/// ```
///
/// ### Persistent-only Mode
/// ```rust,ignore
/// #[amethystate(prefix = "network", mode = "persistent")]
/// pub struct NetworkConfig {
///     #[amestate(default = "localhost".to_string())]
///     pub host: String,
///     #[amestate(default = 8080)]
///     pub port: u16,
/// }
///
/// // Usage:
/// // let mut cfg = NetworkConfig::load_with(&store)?;
/// // cfg.host = "10.0.0.1".to_string(); // Direct field mutation (plain types)
/// // cfg.save_lazy()?;                  // RAM-buffer write (debounced/background)
/// // cfg.save()?;                       // Immediate synchronous flush to disk
/// ```
///
#[proc_macro_attribute]
pub fn amethystate(args: TokenStream, input: TokenStream) -> TokenStream {
    amethystate::amethystate_impl(args, input)
}

/// Declares a migration step, discovered wherever it is written.
///
/// The function takes the old shape and returns the new one; the engine finds
/// it through `StoreBuilder::build_with_migration`, so nothing has to register
/// it by hand. `#[rename(old => new)]` marks the old key for removal once the
/// body has produced the new value, and checks at compile time that both
/// fields exist.
///
/// The macro derives source and target types from the function signature:
/// - **from**: the type of the first argument
/// - **to**: the inner type of the `Result<T>` return type
///
/// The function name becomes the migration step description in the registry.
///
/// # Attributes
///
/// - `#[rename(old_field => new_field)]` — declares a field rename. Can be stacked.
///   Generates a compile-time check that both fields exist on the respective types.
///
/// # Examples
///
/// Simple rename, no context:
///
/// ```rust,ignore
/// mod v1 {
///     #[amethystate(prefix = "app", version = 1)]
///     pub struct Config {
///         #[amestate(default = "localhost".to_string())]
///         pub host: String,
///         #[amestate(default = 8080)]
///         pub port: u16,
///     }
/// }
///
/// #[amethystate(prefix = "app", version = 2)]
/// pub struct Config {
///     #[amestate(default = "localhost".to_string())]
///     pub address: String,
///     #[amestate(default = 8080)]
///     pub port: u16,
/// }
///
/// #[migrate]
/// #[rename(host => address)]
/// fn migrate_config_v1_to_v2(old: AmeData<v1::Config>) -> amethystate::MigrationResult<AmeData<Config>> {
///     Ok(AmeData::<Config> { address: old.host, port: old.port })
/// }
/// ```
///
/// Manual key cleanup via `MigrationContext`:
///
/// ```rust,ignore
/// #[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
/// pub struct ProxyEndpoint {
///     pub url: String,
///     pub timeout_ms: u32,
/// }
///
/// mod v1 {
///     #[amethystate(prefix = "network", version = 1)]
///     pub struct ProxyConfig {
///         #[amestate(default = "default".into())]
///         pub name: String,
///         pub routes: ReactiveMap<String, String>,
///     }
/// }
///
/// #[amethystate(prefix = "network", version = 2)]
/// pub struct ProxyConfig {
///     #[amestate(default = "default".into())]
///     pub name: String,
///     pub endpoints: ReactiveMap<String, ProxyEndpoint>,
/// }
///
/// #[migrate]
/// fn migrate_proxy_config_v1_to_v2(
///     old: AmeData<v1::ProxyConfig>,
///     ctx: &mut amethystate::migration::MigrationContext,
/// ) -> amethystate::MigrationResult<AmeData<ProxyConfig>> {
///     for key in old.routes.keys() {
///         ctx.delete(&format!("routes.{}", key))?;
///     }
///     let endpoints = old.routes.into_iter()
///         .map(|(k, v)| (k, ProxyEndpoint { url: v, timeout_ms: 5000 }))
///         .collect();
///     Ok(AmeData::<ProxyConfig> { name: old.name, endpoints })
/// }
/// ```
///
/// # What a step needs from outside the store
///
/// A step is collected at link time as a bare `fn`, so it captures nothing:
/// anything the application has to hand it - a lookup table, a client, the
/// settings it is porting away from - reaches it through
/// `StoreBuilder::provide`, and is read back by type.
///
/// ```rust,ignore
/// struct LegacyDefaults { port: u16 }
///
/// let (store, report) = StoreBuilder::new(path)
///     .provide(LegacyDefaults { port: 8080 })
///     .build_with_migration()?;
///
/// #[migrate]
/// fn migrate_settings_v1_to_v2(
///     old: AmeData<v1::Settings>,
///     ctx: &mut amethystate::migration::MigrationContext,
/// ) -> amethystate::MigrationResult<AmeData<Settings>> {
///     let legacy = ctx.require::<LegacyDefaults>()?;
///     Ok(AmeData::<Settings> { host: old.host, port: legacy.port })
/// }
/// ```
///
/// `require` fails naming the type when nothing was provided for it;
/// `provided` hands back an `Option` where the step can carry on without it.
#[proc_macro_attribute]
pub fn migrate(args: TokenStream, input: TokenStream) -> TokenStream {
    migrate::migrate_impl(args, input)
}
