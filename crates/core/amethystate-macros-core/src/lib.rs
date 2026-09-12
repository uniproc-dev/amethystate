use darling::FromField;
use darling::util::SpannedValue;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use syn::{GenericArgument, Ident, PathArguments, Type, TypePath, Visibility};

#[derive(Debug, darling::FromMeta, Clone)]
pub struct MacroArgs {
    #[darling(default)]
    pub prefix: Option<SpannedValue<String>>,
    #[darling(default)]
    pub version: Option<u32>,
    #[darling(default)]
    pub mode: Option<String>,
    #[darling(default)]
    pub target: Option<String>,
    #[darling(default)]
    pub as_root: bool,
    /// How every field's own name is spelled where it is stored, said once for
    /// the whole struct. A field with `path` of its own is not touched by it.
    #[darling(default)]
    pub rename_all: Option<SpannedValue<String>>,
    #[darling(default)]
    pub on_unreadable: Option<syn::Path>,
    #[darling(default)]
    pub on_delete: Option<syn::Path>,
    /// What a map does with an entry it cannot read, said once for the whole
    /// struct. Fields that are not maps have no entries and ignore it.
    #[darling(default)]
    pub unreadable_entries: Option<syn::Path>,
    #[darling(default)]
    pub check: Option<syn::Path>,
}

#[derive(Debug, Clone)]
pub struct StoreFieldEntry {
    pub ident: Option<Ident>,
    pub vis: Visibility,
    pub ty: Type,
    /// Where this field is stored, when that is not its own name.
    ///
    /// Written as `path`. A dot in it is a level, so a field can be put
    /// anywhere under the prefix rather than only renamed.
    pub key: Option<SpannedValue<String>>,
    pub default: Option<TokenStream2>,
    pub nested: bool,
    pub volatile: bool,
    /// Whether this field's own paths sit at its holder's level rather than
    /// under a segment named after it.
    ///
    /// Written as `flatten`, and only on a `nested` field: a leaf is one value
    /// at one path and has no paths of its own to merge upward.
    pub flatten: bool,
    pub on_unreadable: Option<syn::Path>,
    pub on_delete: Option<syn::Path>,
    /// What this map does with an entry it cannot read. Only a map has entries,
    /// so it is refused on anything else.
    pub unreadable_entries: Option<syn::Path>,
    pub check: Option<syn::Path>,
    /// The module holding both halves of how this field is stored, named the
    /// way serde names one: `serialize` and `deserialize` inside it.
    pub with: Option<syn::Path>,
    pub serialize_with: Option<syn::Path>,
    pub deserialize_with: Option<syn::Path>,
}

impl StoreFieldEntry {
    /// The function that writes this field, when its own type is not what
    /// writes it.
    pub fn writes_with(&self) -> Option<syn::Path> {
        self.serialize_with.clone().or_else(|| {
            self.with.clone().map(|mut module| {
                module.segments.push(syn::parse_quote!(serialize));
                module
            })
        })
    }

    /// The function that reads it back.
    pub fn reads_with(&self) -> Option<syn::Path> {
        self.deserialize_with.clone().or_else(|| {
            self.with.clone().map(|mut module| {
                module.segments.push(syn::parse_quote!(deserialize));
                module
            })
        })
    }
}

impl StoreFieldEntry {
    /// The name this field is stored under: what `path` says, or the field's own.
    pub fn stored_name(&self) -> String {
        match &self.key {
            Some(key) => key.as_ref().clone(),
            None => self
                .ident
                .as_ref()
                .map(|ident| ident.to_string())
                .unwrap_or_default(),
        }
    }
}

impl FromField for StoreFieldEntry {
    fn from_field(field: &syn::Field) -> darling::Result<Self> {
        let mut entry = StoreFieldEntry {
            ident: field.ident.clone(),
            vis: field.vis.clone(),
            ty: field.ty.clone(),
            key: None,
            default: None,
            nested: false,
            volatile: false,
            flatten: false,
            on_unreadable: None,
            on_delete: None,
            unreadable_entries: None,
            check: None,
            with: None,
            serialize_with: None,
            deserialize_with: None,
        };

        for attr in &field.attrs {
            if attr.path().is_ident("amestate") {
                let list = attr.meta.require_list().map_err(darling::Error::from)?;
                parse_state_tokens(list.tokens.clone(), &mut entry)?;
            }
        }

        Ok(entry)
    }
}

/// The entries an `#[amestate(..)]` holds, split where one ends and the next
/// begins.
///
/// Brackets of every kind arrive as one `Group`, so a comma inside `(..)`,
/// `[..]` or `{..}` is never seen here. Angle brackets are not a group - they
/// are punctuation - so a turbofish is counted: `default =
/// BTreeMap::<String, u32>::new()` is one entry, and taking its comma for a
/// separator would have made it two that neither parses.
///
/// Only a `<` that follows `::` opens one, which is what a turbofish is. A `<`
/// anywhere else in an expression is a comparison, and counting that would go
/// wrong in the other direction.
fn split_top_level_commas(tokens: TokenStream2) -> Vec<TokenStream2> {
    let mut result: Vec<TokenStream2> = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();
    let mut depth = 0usize;
    let mut after_path_sep = false;

    for tt in tokens {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => {
                result.push(current.drain(..).collect());
                after_path_sep = false;
                continue;
            }
            TokenTree::Punct(p) if p.as_char() == '<' && after_path_sep => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' && depth > 0 => depth -= 1,
            _ => {}
        }

        after_path_sep = matches!(&tt, TokenTree::Punct(p) if p.as_char() == ':');
        current.push(tt);
    }

    if !current.is_empty() {
        result.push(current.into_iter().collect());
    }
    result
}

fn parse_state_tokens(tokens: TokenStream2, into: &mut StoreFieldEntry) -> darling::Result<()> {
    for item in split_top_level_commas(tokens) {
        let mut iter = item.into_iter().peekable();

        let first = match iter.next() {
            Some(TokenTree::Ident(i)) => i,
            Some(tt) => {
                return Err(
                    darling::Error::custom("expected attribute key identifier").with_span(&tt)
                );
            }
            None => continue,
        };
        let name = first.to_string();

        let has_eq = matches!(iter.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '=');

        if has_eq {
            iter.next();
            let value: TokenStream2 = iter.collect();

            let already = match name.as_str() {
                "default" => into.default.is_some(),
                "path" => into.key.is_some(),
                "on_unreadable" => into.on_unreadable.is_some(),
                "on_delete" => into.on_delete.is_some(),
                "unreadable_entries" => into.unreadable_entries.is_some(),
                "check" => into.check.is_some(),
                "with" => into.with.is_some(),
                "serialize_with" => into.serialize_with.is_some(),
                "deserialize_with" => into.deserialize_with.is_some(),
                _ => false,
            };

            if already {
                return Err(darling::Error::custom(format!(
                    "`{name}` is said twice. The second would win and the first would look like it \
                     had been read, so write one"
                ))
                .with_span(&first));
            }

            match name.as_str() {
                "default" => into.default = Some(value),
                "path" => {
                    let written: syn::LitStr = syn::parse2(value).map_err(darling::Error::from)?;
                    into.key = Some(SpannedValue::new(written.value(), written.span()));
                }
                "key" => {
                    return Err(darling::Error::custom(
                        "`key` is spelled `path` now: a dot in it is a level, so what it names is where the field sits and not a key it sits under",
                    )
                    .with_span(&first));
                }
                "on_unreadable" => {
                    into.on_unreadable = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "on_delete" => {
                    into.on_delete = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "unreadable_entries" => {
                    into.unreadable_entries =
                        Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "check" => {
                    into.check = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "with" => {
                    into.with = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "serialize_with" => {
                    into.serialize_with = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "deserialize_with" => {
                    into.deserialize_with = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                other => {
                    return Err(darling::Error::unknown_field_with_alts(
                        other,
                        &[
                            "path",
                            "default",
                            "on_unreadable",
                            "on_delete",
                            "unreadable_entries",
                            "check",
                            "with",
                            "serialize_with",
                            "deserialize_with",
                        ],
                    )
                    .with_span(&first));
                }
            }
        } else {
            match name.as_str() {
                "volatile" => into.volatile = true,
                "nested" => into.nested = true,
                "flatten" => into.flatten = true,
                other => {
                    return Err(darling::Error::unknown_field_with_alts(
                        other,
                        &["volatile", "nested", "flatten"],
                    )
                    .with_span(&first));
                }
            }
        }
    }
    Ok(())
}

impl StoreFieldEntry {
    pub fn get_map_types(&self) -> Option<(&Type, &Type)> {
        if let Type::Path(TypePath { path, .. }) = &self.ty {
            let last_seg = path.segments.last()?;
            if last_seg.ident == "ReactiveMap"
                && let PathArguments::AngleBracketed(args) = &last_seg.arguments
            {
                let mut generics = args.args.iter().filter_map(|arg| {
                    if let GenericArgument::Type(t) = arg {
                        Some(t)
                    } else {
                        None
                    }
                });
                let k = generics.next()?;
                let v = generics.next()?;
                return Some((k, v));
            }
        }
        None
    }
}

pub fn get_type_ident_str(ty: &syn::Type) -> String {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident.to_string();
    }
    "any".to_string()
}
