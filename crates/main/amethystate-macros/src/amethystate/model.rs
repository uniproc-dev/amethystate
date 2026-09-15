//! What a declaration means, once every question about it has been answered.
//!
//! `syn` hands over a parse tree, which says what was written. This says what
//! it amounts to: which kind each field is, where it is stored, what it falls
//! back to. Nothing downstream asks the parse tree anything - a generator that
//! could still ask "is this nested?" would be a generator that can forget to.

use proc_macro2::Span;
use syn::{Attribute, Ident, Type, Visibility};

use proc_macro2::TokenStream as TokenStream2;

/// A value and where it was written, so a refusal about it can point there.
#[derive(Debug, Clone)]
pub(crate) struct At<T> {
    pub value: T,
    pub span: Span,
}

impl<T> At<T> {
    pub(crate) fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }
}

/// Which of the three kinds a field is, with what each kind needs.
///
/// One value rather than three questions, so every place that has to tell them
/// apart is a `match` the compiler completes.
///
/// Whether a stored field is one value or a map is not among them. A macro runs
/// before types exist and can only read how a type was spelled, which an alias
/// defeats; the generated code asks `shape::Kind` instead, and the compiler
/// answers for the type itself.
#[derive(Debug, Clone)]
pub(crate) enum Shape {
    /// A value or a map at one path.
    Stored {
        /// What it holds before anything is stored, where a `default` was
        /// written. Where none was, the type's `Seed` is defaulted.
        default: Option<TokenStream2>,
        /// How it is stored, when that is not how its type would be.
        stored_as: Option<StoredAs>,
    },

    /// A struct with paths of its own.
    Node {
        /// Whether its fields sit at this level rather than under a segment
        /// named after the field.
        flattened: bool,
    },

    /// Held in memory and never stored, so it has no path at all.
    Volatile { default: TokenStream2 },
}

/// The key and value types of a map as it was written, `ReactiveMap<K, V>`.
///
/// Only for the generators that turn a declaration into something outside this
/// crate's type system - the TypeScript export and the browser client - which
/// need the names while expanding. Everything else asks `shape::Kind`, which
/// sees through the alias this cannot.
pub(crate) fn written_map(ty: &Type) -> Option<(&Type, &Type)> {
    let Type::Path(syn::TypePath { path, .. }) = ty else {
        return None;
    };

    let last = path.segments.last()?;
    if last.ident != "ReactiveMap" {
        return None;
    }

    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };

    let mut types = args.args.iter().filter_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    });

    Some((types.next()?, types.next()?))
}

/// The pair of functions a field is stored through.
#[derive(Debug, Clone)]
pub(crate) struct StoredAs {
    pub write: Option<syn::Path>,
    pub read: Option<syn::Path>,
}

/// Where a schema's fields hang.
///
/// The top of the store is reachable two ways, and which was written decides
/// whether it is accepted: `as_root` says it, and a prefix naming no level is
/// a prefix that was left unfinished.
#[derive(Debug, Clone)]
pub(crate) enum Placement {
    /// From `as_root`.
    Root,
    /// From `prefix = ..`, as it was written.
    Under(At<String>),
}

impl Placement {
    /// The path this amounts to, which is what the generators are handed.
    pub(crate) fn path(&self) -> String {
        match self {
            Self::Root => super::generate::ROOT.to_string(),
            Self::Under(prefix) => prefix.value.clone(),
        }
    }
}

/// What happens when the store holds something this field cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnUnreadable {
    /// Construction fails and names the path.
    Refuse,
    /// The declared default is taken, and the stored value left where it is.
    UseDefault,
}

/// What a field reports once the key behind it is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnDelete {
    /// The declared default again.
    UseDefault,
    /// The last value it held.
    Keep,
}

/// What a map does with an entry it cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnreadableEntries {
    /// Building the map fails and names the entry.
    Refuse,
    /// The entry is left out and the rest of the map is built.
    Skip,
}

/// What a field does about the store disagreeing with it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Rules {
    pub on_unreadable: Option<At<OnUnreadable>>,
    pub on_delete: Option<At<OnDelete>>,
    pub unreadable_entries: Option<At<UnreadableEntries>>,
    pub check: Option<At<syn::Path>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Field {
    pub ident: Ident,
    pub vis: Visibility,
    pub ty: Type,

    /// The attributes written on this field that are not this macro's own.
    ///
    /// Carried onto the field the generated struct declares, so a doc comment
    /// arrives where it was aimed and an attribute nobody here understands is
    /// judged by whoever does - rather than vanishing because the field is
    /// written out again from scratch.
    pub forwarded: Vec<Attribute>,

    /// The doc comment written on this field, its lines joined.
    ///
    /// A description for whoever reads the code or a dump of it, and not a
    /// label: it is in one language and fixed at compile time, which is what a
    /// catalogue exists to avoid.
    pub described: String,

    /// The name this field is stored under, which is its own unless something
    /// said otherwise. A dot in it is a level.
    pub stored: At<String>,

    pub shape: Shape,
    pub rules: Rules,
}

impl Field {
    /// Whether this field is written to the store at all.
    pub(crate) fn is_stored(&self) -> bool {
        !matches!(self.shape, Shape::Volatile { .. })
    }
}

/// Which halves of the generated code a declaration asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Reactive,
    Persistent,
    Both,
}

impl Mode {
    /// Whether the struct's fields watch the store, which is what everything
    /// reactive is generated for.
    pub(crate) fn watches(self) -> bool {
        matches!(self, Self::Reactive | Self::Both)
    }
}

/// Where the generated code runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// This process, against a store it holds.
    Native,
    /// A browser, against a store on the other side of a Tauri command.
    TauriWasm,
}

/// One `#[amethystate]` declaration, lowered.
#[derive(Debug, Clone)]
pub(crate) struct Schema {
    pub name: Ident,
    pub vis: Visibility,

    /// The attributes to carry onto the generated types: everything the caller
    /// wrote that is not this macro's own vocabulary.
    pub forwarded: Vec<Attribute>,

    /// Where this schema's fields hang, or `None` for a struct meant to be
    /// embedded, which hangs where its holder puts it.
    pub prefix: Option<Placement>,

    pub version: u32,

    /// Which line of declarations at the prefix this is a version of, as
    /// written; `None` for the prefix's unnamed line.
    pub id: Option<At<String>>,

    pub mode: Mode,
    pub target: Target,
    pub rules: Rules,
    pub fields: Vec<Field>,
}

impl Schema {
    /// Whether this schema names a place of its own, rather than taking the
    /// one it is embedded at.
    pub(crate) fn is_root(&self) -> bool {
        self.prefix.is_some()
    }

    /// The fields that reach the store, in the order they were declared.
    pub(crate) fn stored(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.is_stored())
    }
}
