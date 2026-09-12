//! Every rule about a declaration, over the model rather than the parse tree.
//!
//! One place, so a rule is written once and a new one has one home. And one
//! pass, so a struct with three mistakes in it reports three.

use syn::spanned::Spanned;

use super::diagnostics::Diagnostics;
use super::generate::{check_written_path, names_type};
use super::model::{Field, Mode, OnUnreadable, Placement, Schema, Shape, Target};

pub(crate) fn schema(schema: &Schema, found: &mut Diagnostics) {
    if let Some(Placement::Under(prefix)) = &schema.prefix
        && let Err(message) = check_written_path(
            "prefix",
            &prefix.value,
            "; write `as_root` for a struct whose fields sit at the top of the store",
        )
    {
        found.at(prefix.span, message);
    }

    if schema.prefix.is_none() && schema.mode != Mode::Reactive {
        found.at(
            schema.name.span(),
            "a struct with no prefix is a component: it is built inside one that has a place, \
             and its own place is the field holding it. `persistent` gives it a door of its \
             own - `load_with` and `save` - and a door has to open somewhere. Say `prefix = \
             \"..\"`, or `as_root` if the top of the store is where these fields belong",
        );
    }

    if schema.target == Target::TauriWasm && schema.mode != Mode::Reactive {
        found.at(
            schema.name.span(),
            "`tauri-wasm` builds against a store on the other side of a command, and `persistent` \
             is a struct that loads and saves itself against one it holds. There is nothing here \
             to load from: what the browser has is what the last command answered. Drop the mode, \
             and let the fields watch",
        );
    }

    for field in &schema.fields {
        one(schema, field, found);
    }
}

fn one(schema: &Schema, field: &Field, found: &mut Diagnostics) {
    let named = &field.ident;

    if field.is_stored()
        && let Err(message) = check_written_path("name", &field.stored.value, "")
    {
        found.at(field.stored.span, message);
    }

    if let Shape::Node { .. } = field.shape
        && names_type(&field.ty, &schema.name)
    {
        let holder = &schema.name;
        found.at(
            field.ty.span(),
            format!(
                "`{holder}` would build another `{holder}` to build itself, and never stop. A \
                 node that holds one of its own kind has to be able to stop - `Option<Box<_>>` \
                 at `None`, a collection at empty - and neither is built the way a `nested` \
                 field is"
            ),
        );
    }

    if let Some(check) = &field.rules.check {
        let refusal = match &field.shape {
            Shape::Volatile { .. } => Some(format!(
                "`{named}` is volatile, so nothing arrives from the store for a check to judge. \
                 A value this process holds and never stores is the interceptor's business"
            )),
            Shape::Node { .. } => Some(format!(
                "`{named}` is a nested struct, and a check on one belongs on the struct itself - \
                 `#[amethystate(check = ..)]` there is handed every field of it at once, which \
                 is what a rule about a struct needs"
            )),
            Shape::Map { .. } => Some(format!(
                "`{named}` is a map, and its entries are data rather than declared paths: one \
                 bad entry is no reason to withhold the struct, so a map wants dropping and \
                 reporting rather than this"
            )),
            Shape::Leaf { .. } => None,
        };

        if let Some(message) = refusal {
            found.at(check.span, message);
        }
    }

    weaker_than_the_struct(schema, field, found);
}

/// A field may demand more of the store than its struct did, and never less.
fn weaker_than_the_struct(schema: &Schema, field: &Field, found: &mut Diagnostics) {
    let (Some(struct_rule), Some(field_rule)) =
        (&schema.rules.on_unreadable, &field.rules.on_unreadable)
    else {
        return;
    };

    if struct_rule.value == OnUnreadable::Refuse && field_rule.value == OnUnreadable::UseDefault {
        let holder = &schema.name;
        let field = &field.ident;
        found.at(
            field_rule.span,
            format!(
                "`{holder}` declares `on_unreadable = Refuse`, so `{field}` cannot ask for \
                 `UseDefault`. A field may demand more than the struct promised and never less: \
                 drop this to inherit the struct's rule, or move `UseDefault` up to the struct \
                 and write `Refuse` on the fields that must be readable"
            ),
        );
    }
}
