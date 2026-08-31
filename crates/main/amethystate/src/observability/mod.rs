mod inspector_trait;
mod scheme;
pub use inspector_trait::*;

pub use scheme::*;

use amethystate_core::path::StorePath;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};
use uuid::Uuid;

pub fn short_type_name(full: &str) -> &str {
    full.rsplit("::").next().unwrap_or(full)
}

#[derive(Debug, Clone)]
pub struct FieldMeta {
    pub struct_type_name: &'static str,
    pub field_name: Arc<str>,

    /// What the value's type is, for an inspector to show. Nothing decides
    /// anything by it.
    pub value_type_name: &'static str,
}

static INSTANCE_REGISTRY: LazyLock<RwLock<HashMap<Uuid, &'static str>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

static SCHEMA_REGISTRY: LazyLock<RwLock<HashMap<StorePath, FieldMeta>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn register_instance(id: Uuid, struct_type_name: &'static str) {
    if let Ok(mut map) = INSTANCE_REGISTRY.write() {
        map.insert(id, struct_type_name);
    }
}

/// Keeps an instance in the registry for as long as any clone of the state
/// struct is alive, and drops it from the registry when the last one goes.
pub struct InstanceGuard {
    id: Uuid,
}

impl InstanceGuard {
    pub fn new(id: Uuid, struct_type_name: &'static str) -> Arc<Self> {
        register_instance(id, struct_type_name);
        Arc::new(Self { id })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        deregister_instance(self.id);
    }
}

pub fn deregister_instance(id: Uuid) {
    if let Ok(mut map) = INSTANCE_REGISTRY.write() {
        map.remove(&id);
    }
}

pub fn resolve_instance(id: Uuid) -> Option<&'static str> {
    INSTANCE_REGISTRY.read().ok()?.get(&id).copied()
}

pub fn resolve_instance_short(id: Uuid) -> Option<&'static str> {
    resolve_instance(id).map(short_type_name)
}

pub fn register_field<T: 'static>(path: &StorePath, instance_id: Uuid) {
    let struct_type_name = match resolve_instance(instance_id) {
        Some(n) => n,
        None => return,
    };
    let field_name: Arc<str> = match path.name() {
        Some(name) => Arc::from(name.as_ref()),
        None => return,
    };
    if let Ok(mut map) = SCHEMA_REGISTRY.write() {
        map.entry(path.clone()).or_insert(FieldMeta {
            struct_type_name,
            field_name,
            value_type_name: std::any::type_name::<T>(),
        });
    }
}

pub fn resolve_field(path: &str) -> Option<FieldMeta> {
    SCHEMA_REGISTRY.read().ok()?.get(path).cloned()
}
