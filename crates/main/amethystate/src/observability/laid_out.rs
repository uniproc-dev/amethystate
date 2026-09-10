//! What a declared struct looks like when somebody is looking at it.
//!
//! One table: the name in the source, where it is stored, what it was declared
//! as, what it holds, and whatever the store disagrees with it about. Nested
//! structs are indented under the field that holds them.
//!
//! ```ignore
//! println!("{}", editor.inspect());
//! ```
//!
//! For a bug report, a terminal, or the moment somebody asks "what does it
//! think it has".

use std::fmt;

use crate::observability::Inspect;

/// A struct rendered as a table, produced by [`InspectExt::inspect`].
pub struct LaidOut<'a> {
    of: &'a dyn Inspect,
}

/// `inspect()` on anything that can list its fields.
pub trait InspectExt: Inspect {
    /// This struct as a table, for showing to a person.
    fn inspect(&self) -> LaidOut<'_>
    where
        Self: Sized,
    {
        LaidOut { of: self }
    }
}

impl<T: Inspect> InspectExt for T {}

impl fmt::Display for LaidOut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        rows(f, self.of, 0)
    }
}

fn rows(f: &mut fmt::Formatter<'_>, of: &dyn Inspect, depth: usize) -> fmt::Result {
    let indent = "  ".repeat(depth);

    for index in 0..of.field_count() {
        let Some(view) = of.field_at(index) else {
            break;
        };

        writeln!(
            f,
            "{indent}{name:<20} {at:<28} {ty:<22} {shown}",
            name = view.declared,
            at = view.at,
            ty = view.type_name,
            shown = view.shown,
        )?;

        if let Some(gone) = &view.disagreement {
            writeln!(f, "{indent}  ! {}", gone.reason)?;
        }

        if let Some(inside) = view.inside {
            rows(f, inside, depth + 1)?;
        }
    }

    Ok(())
}
