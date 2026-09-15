//! Which struct an instance is, for as long as one is alive.
//!
//! A claim belongs to a name rather than to a handle: the same struct built
//! twice claims the same places twice and changes nothing, and two different
//! structs reaching one place is the collision
//! [`Places`](crate::store::places::Places) refuses. Neither is answerable
//! from a `Uuid` alone, so this is what turns one back into the name the claim
//! is attributed to.
//!
//! Which is why it sits with the store rather than with the reporting. It is
//! not a thing shown to anybody: it is what makes a refusal name both sides,
//! and an instance nobody registered claims nothing at all - there is no name
//! to attribute it to, and refusing what cannot be attributed would be
//! guessing.

use dashmap::DashMap;
use std::sync::{Arc, LazyLock};
use uuid::Uuid;

/// Sharded rather than one lock, because nothing here walks it: every use is a
/// single id looked up, put in or taken out. Iterating a `DashMap` is the
/// thing to be careful about, and there is nothing to iterate.
static INSTANCES: LazyLock<DashMap<Uuid, &'static str>> = LazyLock::new(DashMap::new);

pub fn short_type_name(full: &str) -> &str {
    full.rsplit("::").next().unwrap_or(full)
}

pub fn register_instance(id: Uuid, struct_type_name: &'static str) {
    INSTANCES.insert(id, struct_type_name);
}

pub fn deregister_instance(id: Uuid) {
    INSTANCES.remove(&id);
}

pub fn resolve_instance(id: Uuid) -> Option<&'static str> {
    INSTANCES.get(&id).map(|found| *found)
}

pub fn resolve_instance_short(id: Uuid) -> Option<&'static str> {
    resolve_instance(id).map(short_type_name)
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
