use crate::Store;
use crate::migration::fields::AmeStateFields;
use crate::store::StorageResult;
use amethystate_core::path::StorePath;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub trait AmeStateNode: Sized {
    /// Nothing to read: evaluating it is the check.
    ///
    /// Constructing a node constructs every node it declares - a `nested`
    /// field - so a cycle among them is a constructor that never returns. The
    /// generated value reads this same constant from each of
    /// those types, which makes a cycle a cycle in const evaluation, and that is
    /// a compile error rather than a stack overflow at startup.
    ///
    /// What counts as a cycle is a reference the constructor always follows.
    /// Recursion that can bottom out is not one: an `Option<Box<_>>` stops at
    /// `None`, a collection stops at empty, and a map recursing through its
    /// value type never goes through a constructor at all - those values are
    /// decoded. So the edges are the unconditional ones, and only those.
    ///
    /// Kept apart from `TYPE_HASH` on purpose. The hash says what a struct
    /// stores, and folding a linked node's shape into it would send a struct
    /// through a migration because something it only reads from changed.
    ///
    /// Required rather than defaulted: an impl written by hand would otherwise
    /// take an empty default and opt out without saying so, and what it opts
    /// out of is a crash with no message.
    const CONSTRUCTION_TERMINATES: ();

    fn new_node(store: &Store, path: &StorePath) -> StorageResult<Self>;
    fn new_node_with_id(store: &Store, path: &StorePath, instance_id: Uuid) -> StorageResult<Self>;
}

pub trait AmeState {
    type Data: AmeStateFields
        + Serialize
        + for<'de> Deserialize<'de>
        + Clone
        + Send
        + Sync
        + 'static;
}
