#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Plain,
    Nested {
        struct_name: &'static str,
    },
    Volatile,
    ReactiveMap {
        key_type: &'static str,
        value_type: &'static str,
        key_rust_type: &'static str,
        value_rust_type: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldExportMeta {
    pub name: &'static str,
    pub ts_type: &'static str,
    pub full_ts_type: &'static str,
    pub rust_type: &'static str,
    pub kind: FieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaExportEntry {
    pub prefix: Option<&'static str>,
    pub struct_name: &'static str,
    pub fields: &'static [FieldExportMeta],
}

inventory::collect!(SchemaExportEntry);

/// Every struct this binary offers a generator.
///
/// The one reader of this list, the way `schema::declarations` is of the one
/// the store reads: it is fixed for the life of the process, so it is walked on
/// the first ask and handed out as a slice afterwards.
pub fn exports() -> &'static [&'static SchemaExportEntry] {
    static COMPILED: std::sync::OnceLock<Vec<&'static SchemaExportEntry>> =
        std::sync::OnceLock::new();

    COMPILED.get_or_init(|| inventory::iter::<SchemaExportEntry>.into_iter().collect())
}
