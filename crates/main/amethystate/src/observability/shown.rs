//! Showing a value whose type may or may not be showable.
//!
//! A declared field holds whatever its type is, and nothing requires that type
//! to implement `Debug`. Generated code still has to print it - in the struct's
//! own `Debug`, and in [`FieldView::value`](crate::observability::FieldView).
//!
//! The pick happens at compile time and costs nothing at runtime. An inherent
//! method on `Shown<T>` is reached before one on `&Shown<T>`, so a `T` that
//! implements `Debug` takes the first and everything else falls to the second -
//! which is how a trait bound is asked about without a `where` clause to say it
//! in.

use std::fmt;

/// What stands in for a value whose type cannot be printed.
pub struct Opaque;

impl fmt::Debug for Opaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<opaque>")
    }
}

/// The one that stands in for every value that cannot be printed.
///
/// A `static` rather than a temporary, because what is handed back outlives
/// the `Shown` that was asked.
static OPAQUE: Opaque = Opaque;

/// A value being asked whether it can be shown.
pub struct Shown<'a, T>(pub &'a T);

pub trait ShownByDebug<'a> {
    fn shown(&self) -> &'a dyn fmt::Debug;
}

impl<'a, T: fmt::Debug> ShownByDebug<'a> for Shown<'a, T> {
    fn shown(&self) -> &'a dyn fmt::Debug {
        self.0
    }
}

pub trait ShownAsOpaque<'a> {
    fn shown(&self) -> &'a dyn fmt::Debug;
}

impl<'a, T> ShownAsOpaque<'a> for &Shown<'a, T> {
    fn shown(&self) -> &'a dyn fmt::Debug {
        &OPAQUE
    }
}
