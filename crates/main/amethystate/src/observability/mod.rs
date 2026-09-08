//! What a running program can say about its own state.
//!
//! Only reporting. Nothing here decides anything: a store opens, claims and
//! migrates without asking this module a question, and what it holds is what a
//! screen, a log line or a dump needs in order to name a field the way the
//! person who wrote it would.
//!
//! Which is why the neighbouring questions are answered elsewhere, by whoever
//! acts on them: the declarations this binary carries are
//! [`schema`](crate::schema), which the migration engine and `Kv` run on;
//! which struct an instance belongs to is
//! [`store::instances`](crate::store::instances), because a claim is
//! attributed by it; and reading a store from outside the program that wrote
//! it is [`store::InspectorBackend`](crate::store::InspectorBackend), which
//! the engines implement.

mod introspect;
mod laid_out;
mod shown;
pub use introspect::*;
pub use laid_out::*;
pub use shown::*;

use crate::store::instances::resolve_instance;
use amethystate_core::path::StorePath;
use dashmap::DashMap;
use std::sync::{Arc, LazyLock};
use uuid::Uuid;

pub use crate::store::instances::short_type_name;

/// What a field is called, and by whom, for something showing it.
#[derive(Debug, Clone)]
pub struct FieldMeta {
    pub struct_type_name: &'static str,
    pub field_name: Arc<str>,

    /// What the value's type is, for an inspector to show. Nothing decides
    /// anything by it.
    pub value_type_name: &'static str,
}

/// Sharded rather than one lock, for the same reason as
/// [`instances`](crate::store::instances): every use is one path, and nothing
/// walks it.
static FIELDS: LazyLock<DashMap<StorePath, FieldMeta>> = LazyLock::new(DashMap::new);

pub fn register_field<T: 'static>(path: &StorePath, instance_id: Uuid) {
    let struct_type_name = match resolve_instance(instance_id) {
        Some(n) => n,
        None => return,
    };
    let field_name: Arc<str> = match path.name() {
        Some(name) => Arc::from(name.as_ref()),
        None => return,
    };

    FIELDS.entry(path.clone()).or_insert(FieldMeta {
        struct_type_name,
        field_name,
        value_type_name: std::any::type_name::<T>(),
    });
}

pub fn resolve_field(path: &str) -> Option<FieldMeta> {
    FIELDS.get(path).map(|found| found.clone())
}
