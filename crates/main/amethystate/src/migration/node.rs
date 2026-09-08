use crate::migration::fields::AmeStateFields;
use serde::{Deserialize, Serialize};

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
    /// Required rather than defaulted: an impl written by hand would otherwise
    /// take an empty default and opt out without saying so, and what it opts
    /// out of is a crash with no message.
    const CONSTRUCTION_TERMINATES: ();
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
