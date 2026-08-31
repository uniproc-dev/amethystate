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
    #[darling(default)]
    pub on_unreadable: Option<syn::Path>,
    #[darling(default)]
    pub on_delete: Option<syn::Path>,
    #[darling(default)]
    pub check: Option<syn::Path>,
}

#[derive(Debug, Clone)]
pub struct StoreFieldEntry {
    pub ident: Option<Ident>,
    pub vis: Visibility,
    pub ty: Type,
    pub key: Option<SpannedValue<String>>,
    pub default: Option<TokenStream2>,
    pub nested: bool,
    pub volatile: bool,
    pub on_unreadable: Option<syn::Path>,
    pub on_delete: Option<syn::Path>,
    pub check: Option<syn::Path>,
}

impl StoreFieldEntry {
    /// The name this field is stored under: what `key` says, or the field's own.
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
            on_unreadable: None,
            on_delete: None,
            check: None,
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

fn split_top_level_commas(tokens: TokenStream2) -> Vec<TokenStream2> {
    let mut result: Vec<TokenStream2> = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();
    for tt in tokens {
        if matches!(&tt, TokenTree::Punct(p) if p.as_char() == ',') {
            result.push(current.drain(..).collect());
        } else {
            current.push(tt);
        }
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

            match name.as_str() {
                "default" => into.default = Some(value),
                "key" => {
                    let lit: syn::LitStr = syn::parse2(value).map_err(darling::Error::from)?;
                    into.key = Some(SpannedValue::new(lit.value(), lit.span()));
                }
                "on_unreadable" => {
                    into.on_unreadable = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "on_delete" => {
                    into.on_delete = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                "check" => {
                    into.check = Some(syn::parse2(value).map_err(darling::Error::from)?);
                }
                other => {
                    return Err(darling::Error::unknown_field_with_alts(
                        other,
                        &["default", "key", "on_unreadable", "on_delete", "check"],
                    ));
                }
            }
        } else {
            match name.as_str() {
                "volatile" => into.volatile = true,
                "nested" => into.nested = true,
                other => {
                    return Err(darling::Error::unknown_field_with_alts(
                        other,
                        &["volatile", "nested"],
                    ));
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
