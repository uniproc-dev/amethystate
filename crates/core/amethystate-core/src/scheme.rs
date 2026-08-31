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
