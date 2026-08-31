use proc_macro::TokenStream;

mod amethystate;
mod hash;
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
/// * `#[amethystate(prefix = "path", version = 1, mode = "reactive", as_root)]` - Defines a **Root** struct.
///   * `as_root` (optional flag): If specified, fields are written directly to the store root without
///     a namespace.
///   * `prefix` (String): Sets the top-level namespace path in the store.
///     Generates `pub fn new() -> StorageResult<Self>`, which opens on the
///     global store, and `pub fn new_with(store: &Store) -> StorageResult<Self>`
///     for a store the caller holds.
///   * `version` (optional u32): Schema version for migrations (defaults to 0).
///   * `mode` (optional String): Controls the generated code paradigm. One of:
///     * `"reactive"` (default): Generates fine-grained reactive `Field<T>` accessors.
///     * `"persistent"`: Generates a flat struct with plain-type fields and synchronous `.save()` / `.save_lazy()` methods.
///     * `"both"`: Generates both reactive accessors on `#name` and a separate `#name_Persistent` flat struct.
///   * `check` (optional path): A `fn(&Data, &CheckContext) -> Result<(), Invalid>`
///     run over the whole struct as it is built.
///   * `on_unreadable` / `on_delete` (optional paths): What every field of this
///     struct falls back to when it says nothing itself - see
///     `store::OnUnreadable` and `store::OnDelete`.
/// * `#[amethystate]` - Defines a **Nested** struct.
///   * Used as a component within other structures.
///   * Generates `pub fn new(store: &Store, namespace: impl IntoStorePath) -> StorageResult<Self>`.
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
/// | the same, with `#[amestate(key = "listen_port")]` | `net.listen_port` |
/// | nested struct at field `db` inside prefix `sys`, its field `host` | `sys.db.host` |
/// | `#[amethystate(as_root)]`, field `port` | `port` |
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
/// | `default` | `= Expr` | Initial value if not present in store. Required for leaf fields. |
/// | `key` | `= String` | Overrides the storage key (defaults to field name). |
/// | `check` | `= path` | A `fn(&T, &CheckContext) -> Result<(), Invalid>` every value coming in from the store has to pass. |
/// | `on_unreadable` | `= path` | What this field does about a stored value it will not accept - see `store::OnUnreadable`. |
/// | `on_delete` | `= path` | What this field does when its key is deleted under it - see `store::OnDelete`. |
/// | `nested` | flag | Marks field as another `#[amethystate]` struct. |
/// | `volatile` | flag | In-memory only. Never saved to or loaded from disk. |
///
/// `nested` and `volatile` are bare flags: they are written on their own, with
/// no `= true`.
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
/// // let settings = AppSettings::new(&store)?;
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
/// // let mut cfg = NetworkConfig::load(&store)?;
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
/// it by hand. `#[rename(old => new)]` moves a key whose value survives
/// unchanged, so the body does not have to copy it.
///
/// ```rust,ignore
/// mod v1 {
///     #[amethystate(prefix = "app", version = 1)]
///     pub struct Config {
///         #[amestate(default = "localhost".to_string())]
///         pub host: String,
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
/// fn config_v1_to_v2(old: AmeData<v1::Config>) -> MigrationResult<AmeData<Config>> {
///     Ok(AmeData::<Config> { address: old.host, port: 9090 })
/// }
/// ```
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
/// #[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, AmeType)]
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

/// Derives the `AmeType` trait for a struct, providing compile-time schema hashing and identification.
///
/// This macro automatically generates a unique schema hash (`TYPE_HASH`) and a string identification
/// name (`TYPE_NAME`) at compile time. It is used by the migration and persistence systems
/// to detect schema changes.
///
/// # Hash Calculation Behavior
///
/// - **Recursive**: The `TYPE_HASH` is calculated recursively based on the name and type of every
///   field inside the struct. Therefore, all fields must also implement the `AmeType` trait.
/// - **Structural (Rename-Compatible)**: The name of the struct itself is **excluded** from the
///   `TYPE_HASH`. This guarantees that renaming a struct in Rust code does not alter its database
///   compatibility or trigger false-positive migrations, as long as its fields and their types
///   remain identical.
///
/// # Examples
///
/// Simple struct:
///
/// ```rust,ignore
/// #[derive(AmeType)]
/// pub struct Endpoint {
///     pub host: String,
///     pub port: u16,
/// }
/// ```
///
/// Nested struct (both must derive `AmeType` to calculate the recursive hash):
///
/// ```rust,ignore
/// #[derive(AmeType)]
/// pub struct DbConfig {
///     pub username: String,
///     pub endpoint: Endpoint, // Recursive hash calculation will include Endpoint's fields
/// }
/// ```
#[proc_macro_derive(AmeType)]
pub fn ame_type_derive(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;
    let crate_name = amethystate::amethystate_crate_path();

    let fields_info = if let syn::Data::Struct(s) = &input.data {
        s.fields
            .iter()
            .map(|f| {
                let field_name = f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                let ty = &f.ty;
                (field_name, quote::quote!(#ty))
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };

    let type_hash_expr = hash::gen_recursive_type_hash(&crate_name, fields_info.clone());

    let schema_export = if cfg!(feature = "tauri") {
        let struct_name_str = name.to_string();
        let field_metas = if let syn::Data::Struct(s) = &input.data {
            s.fields
                .iter()
                .map(|f| {
                    let field_name = f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                    let ty = &f.ty;
                    let (ts_type, full_ts_type) = ts_mapping::map_type_to_ts(ty.clone());
                    let rust_type_str = quote::quote!(#ty).to_string();

                    let kind_tokens = if ts_mapping::is_primitive_ts_type(&ts_type) {
                        quote::quote! { #crate_name::tauri::FieldKind::Plain }
                    } else {
                        quote::quote! { #crate_name::tauri::FieldKind::Nested { struct_name: #ts_type } }
                    };

                    quote::quote! {
                        #crate_name::tauri::FieldExportMeta {
                            name: #field_name,
                            ts_type: #ts_type,
                            full_ts_type: #full_ts_type,
                            rust_type: #rust_type_str,
                            kind: #kind_tokens,
                        }
                    }
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        quote::quote! {
            #crate_name::inventory::submit! {
                #crate_name::tauri::SchemaExportEntry {
                    prefix: None,
                    struct_name: #struct_name_str,
                    fields: &[
                        #(#field_metas),*
                    ],
                }
            }
        }
    } else {
        quote::quote! {}
    };

    let expanded = quote::quote! {
        impl #crate_name::migration::types::AmeType for #name {
            const TYPE_HASH: u32 = #type_hash_expr;
            const TYPE_NAME: &'static str = stringify!(#name);
        }

        #schema_export
    };
    TokenStream::from(expanded)
}
