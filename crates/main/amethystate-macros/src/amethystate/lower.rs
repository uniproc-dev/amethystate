//! Parse tree to model.
//!
//! Every question a generator might have asked is answered here, once: which
//! kind a field is, what it is stored under, what it falls back to. Nothing
//! below this reads `syn` for meaning - only for types it prints back out.
//!
//! Answers no question twice, and stops at nothing: what is wrong is gathered
//! and reported together.

use amethystate_macros_core::{MacroArgs, StoreFieldEntry};
use darling::FromField;
use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{Data, DataStruct, DeriveInput, Fields};

use super::diagnostics::Diagnostics;
use super::model::{
    At, Field, Mode, OnDelete, OnUnreadable, Placement, Rules, Schema, Shape, StoredAs, Target,
};
use super::naming;
use darling::util::SpannedValue;

/// Builds the model, adding what it finds wrong to `found` rather than
/// stopping.
///
/// Only a struct that is not one at all ends it here: a field that would not
/// lower is left out and the rest carry on, so checking still sees them and
/// one compile reports the lot.
pub(crate) fn schema(
    input: &DeriveInput,
    args: &MacroArgs,
    found: &mut Diagnostics,
) -> Result<Schema, syn::Error> {
    if let Some(rule) = &args.rename_all
        && naming::apply(rule.as_ref(), "a_field").is_none()
    {
        let written = rule.as_ref();
        let known = naming::RULES.join("`, `");
        found.at(
            rule.span(),
            format!("`{written}` is not a naming rule. These are: `{known}`"),
        );
    }

    let Data::Struct(DataStruct {
        fields: Fields::Named(named),
        ..
    }) = &input.data
    else {
        found.at(
            input.ident.span(),
            "amethystate can only be used on structs with named fields: it turns each field into \
             a path, and a field with no name has nothing to be called",
        );
        return Err(syn::Error::new(
            input.ident.span(),
            "amethystate can only be used on structs with named fields",
        ));
    };

    let mut fields = Vec::with_capacity(named.named.len());
    for field in &named.named {
        if let Some(lowered) = lower_field(field, args.rename_all.as_ref(), found) {
            fields.push(lowered);
        }
    }

    let prefix = prefix_of(args, found);
    let mode = mode_of(args, found);
    let target = target_of(args, found);
    let rules = rules_of(
        args.on_unreadable.as_ref(),
        args.on_delete.as_ref(),
        args.check.as_ref(),
        found,
    );

    let schema = Schema {
        name: input.ident.clone(),
        vis: input.vis.clone(),
        forwarded: input.attrs.clone(),
        prefix,
        version: args.version.unwrap_or(0),
        mode,
        target,
        rules,
        fields,
    };

    Ok(schema)
}

/// The attributes on a field that are not this macro's own, to be carried onto
/// the field it generates.
///
/// Everything is carried, understood or not: what this macro does not read is
/// somebody else's to judge, and a field written out from scratch would drop it
/// without a word.
///
/// `cfg` is the exception. It says the field may not exist, and a field appears
/// in a dozen places here - the struct, the constructor, the snapshot, the
/// schema on disk - which would have to agree. Carried only to some of them it
/// would build a struct without the field and a schema with it, so it is
/// refused until every place agrees.
fn forwarded_from(field: &syn::Field, found: &mut Diagnostics) -> Vec<syn::Attribute> {
    let mut carried = Vec::new();

    for attr in &field.attrs {
        if attr.path().is_ident("amestate") {
            continue;
        }

        if attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr") {
            found.at(
                attr.span(),
                "a field that may not exist is not carried through here yet: the struct, its \
                 constructor and the schema written to disk would have to agree about it, and \
                 they are built apart. Put the whole struct behind the `cfg`, or keep the field \
                 and decide at runtime what it holds",
            );
            continue;
        }

        carried.push(attr.clone());
    }

    carried
}

/// The doc comment on a field, as one string.
///
/// `///` is `#[doc = ".."]` by the time this sees it, one attribute per line,
/// each with the leading space rustdoc puts there. Joined with newlines, so a
/// paragraph stays a paragraph.
fn described_by(field: &syn::Field) -> String {
    let lines: Vec<String> = field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(named) => Some(&named.value),
            _ => None,
        })
        .filter_map(|value| match value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            }) => Some(text.value().trim().to_string()),
            _ => None,
        })
        .collect();

    lines.join("\n")
}

fn spanned_path(path: Option<&syn::Path>) -> Option<At<syn::Path>> {
    path.map(|p| At::new(p.clone(), p.span()))
}

fn rules_of(
    on_unreadable: Option<&syn::Path>,
    on_delete: Option<&syn::Path>,
    check: Option<&syn::Path>,
    found: &mut Diagnostics,
) -> Rules {
    Rules {
        on_unreadable: variant(
            on_unreadable,
            &[
                ("Refuse", OnUnreadable::Refuse),
                ("UseDefault", OnUnreadable::UseDefault),
            ],
            "`Refuse` is what happens without one: construction fails and names the path. \
             `UseDefault` takes the declared default and carries on, leaving the stored value \
             where it is for somebody to fix, with `try_get` saying so until they do",
            found,
        ),
        on_delete: variant(
            on_delete,
            &[
                ("UseDefault", OnDelete::UseDefault),
                ("Keep", OnDelete::Keep),
            ],
            "`UseDefault` is what happens without one: the field reports its declared default \
             again. `Keep` goes on reporting the last value it held, which is what a value being \
             drawn wants when something else removed the key",
            found,
        ),
        check: spanned_path(check),
    }
}

/// The variant a rule names, as the value it stands for.
///
/// The name is taken from the last segment, so `UseDefault` and
/// `OnUnreadable::UseDefault` both write the same thing and neither needs the
/// type in scope. A name that is neither is refused here rather than becoming
/// whichever variant a later `match` reaches for.
fn variant<T: Copy>(
    written: Option<&syn::Path>,
    allowed: &[(&str, T)],
    hint: &str,
    found: &mut Diagnostics,
) -> Option<At<T>> {
    let path = written?;
    let named = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default();

    match allowed.iter().find(|(name, _)| *name == named) {
        Some((_, value)) => Some(At::new(*value, path.span())),
        None => {
            found.at(
                path.span(),
                format!("`{named}` is not one of these. {hint}"),
            );
            None
        }
    }
}

/// The place this schema's fields hang under.
///
/// `as_root` and `prefix` are two ways of saying it, and saying both leaves
/// nothing to decide which won.
fn prefix_of(args: &MacroArgs, found: &mut Diagnostics) -> Option<Placement> {
    match (&args.prefix, args.as_root) {
        (Some(prefix), true) => {
            found.at(
                prefix.span(),
                "`as_root` puts these fields at the top of the store and `prefix` puts them under \
                 a name, so the two cannot both be true. Drop one",
            );
            None
        }
        (Some(prefix), false) => Some(Placement::Under(At::new(
            prefix.as_ref().clone(),
            prefix.span(),
        ))),
        (None, true) => Some(Placement::Root),
        (None, false) => None,
    }
}

fn mode_of(args: &MacroArgs, found: &mut Diagnostics) -> Mode {
    match args.mode.as_deref() {
        None | Some("reactive") => Mode::Reactive,
        Some("persistent") => Mode::Persistent,
        Some("both") => Mode::Both,
        Some(other) => {
            found.at(
                Span::call_site(),
                format!(
                    "`{other}` is not a mode. `reactive` gives fields that watch the store, \
                     `persistent` gives a struct that loads and saves whole, and `both` gives \
                     each of them"
                ),
            );
            Mode::Reactive
        }
    }
}

fn target_of(args: &MacroArgs, found: &mut Diagnostics) -> Target {
    match args.target.as_deref() {
        None | Some("native") => Target::Native,
        Some("tauri-wasm") => Target::TauriWasm,
        Some(other) => {
            found.at(
                Span::call_site(),
                format!(
                    "`{other}` is not a target. `native` builds against a store this process \
                     holds, and `tauri-wasm` against one on the other side of a Tauri command"
                ),
            );
            Target::Native
        }
    }
}

fn lower_field(
    field: &syn::Field,
    naming: Option<&SpannedValue<String>>,
    found: &mut Diagnostics,
) -> Option<Field> {
    let entry = match StoreFieldEntry::from_field(field) {
        Ok(entry) => entry,
        Err(e) => {
            found.push(e.into());
            return None;
        }
    };

    let ident = entry.ident.clone()?;
    let stored = stored_name(&entry, &ident, naming);
    let forwarded = forwarded_from(field, found);
    let described = described_by(field);

    let rules = rules_of(
        entry.on_unreadable.as_ref(),
        entry.on_delete.as_ref(),
        entry.check.as_ref(),
        found,
    );

    let lowered = Field {
        shape: shape_of(&entry, found),
        rules,
        vis: entry.vis.clone(),
        ty: entry.ty.clone(),
        ident,
        stored,
        forwarded,
        described,
    };

    Some(lowered)
}

/// Where this field is stored: what `path` says, or its own name under the
/// struct's `rename_all`, or its own name.
///
/// A field that named its place is not touched by `rename_all`: naming it was
/// the more specific thing to say.
fn stored_name(
    entry: &StoreFieldEntry,
    ident: &syn::Ident,
    naming: Option<&SpannedValue<String>>,
) -> At<String> {
    if let Some(written) = &entry.key {
        return At::new(written.as_ref().clone(), written.span());
    }

    let named = ident.to_string();

    match naming {
        Some(rule) => At::new(
            naming::apply(rule.as_ref(), &named).unwrap_or(named),
            ident.span(),
        ),
        None => At::new(named, ident.span()),
    }
}

/// Which of the four kinds this field is.
///
/// Decided once, here, so that everything downstream matches rather than asks.
fn shape_of(entry: &StoreFieldEntry, found: &mut Diagnostics) -> Shape {
    let default = entry.default.as_ref().map(super::generate::parse_default);

    let named = entry
        .ident
        .as_ref()
        .map_or_else(|| entry.stored_name(), syn::Ident::to_string);

    if entry.flatten {
        if !entry.nested {
            found.at(
                entry.ty.span(),
                format!(
                    "`{named}` is one value at one path, and `flatten` puts a field's own paths at \
                     the level above. This one has none of its own: the store hands the value to \
                     the codec whole and never looks inside. Flatten belongs on a `nested` field"
                ),
            );
        }

        if let Some(written) = &entry.key {
            found.at(
                written.span(),
                format!(
                    "`{named}` is both placed and not: `path` says where it sits, and `flatten` \
                     says its fields sit at this level with no segment of its own"
                ),
            );
        }
    }

    if entry.volatile {
        if let Some(written) = &entry.key {
            found.at(
                written.span(),
                format!(
                    "`{named}` names where it is stored, and it is stored nowhere: `volatile` \
                     keeps it in memory and gives it no path. Drop one of the two"
                ),
            );
        }

        if entry.nested {
            found.at(
                entry.ty.span(),
                format!(
                    "`{named}` is both `volatile` and `nested`, and the two want opposite things: \
                     `nested` gives a struct whose every field is a path, and `volatile` gives no \
                     path at all. A struct this process holds and never stores is a plain field \
                     of that type"
                ),
            );
        } else if entry.get_map_types().is_some() {
            found.at(
                entry.ty.span(),
                format!(
                    "`{named}` is a `volatile` map, and a `ReactiveMap` is entries under a path: \
                     it is built against the store and has nowhere else to keep them. A map this \
                     process holds and never stores is a plain field over `HashMap`"
                ),
            );
        }

        return Shape::Volatile {
            default: default.unwrap_or_else(|| fallback(&entry.ty)),
        };
    }

    if entry.nested {
        return Shape::Node {
            flattened: entry.flatten,
        };
    }

    if let Some((key, value)) = entry.get_map_types() {
        return Shape::Map {
            key: Box::new(key.clone()),
            value: Box::new(value.clone()),
            default,
        };
    }

    let stored_as = match (entry.writes_with(), entry.reads_with()) {
        (None, None) => None,
        (write, read) => Some(StoredAs { write, read }),
    };

    Shape::Leaf {
        default: default.unwrap_or_else(|| fallback(&entry.ty)),
        stored_as,
    }
}

/// What a field with nothing written falls back to.
fn fallback(ty: &syn::Type) -> proc_macro2::TokenStream {
    quote::quote! { <#ty as ::std::default::Default>::default() }
}
