pub fn map_type_to_ts(ty: syn::Type) -> (String, String) {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let ident_str = segment.ident.to_string();
                match ident_str.as_str() {
                    "String" => ("string".to_string(), "string".to_string()),
                    "bool" => ("boolean".to_string(), "boolean".to_string()),
                    "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32"
                    | "i64" | "i128" | "isize" | "f32" | "f64" => {
                        ("number".to_string(), "number".to_string())
                    }
                    "Vec" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            let (inner_base, inner_full) = map_type_to_ts(inner_ty.clone());
                            return (inner_base, format!("{}[]", inner_full));
                        }
                        ("any".to_string(), "any[]".to_string())
                    }
                    "Option" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            let (inner_base, inner_full) = map_type_to_ts(inner_ty.clone());
                            return (inner_base, format!("{} | null", inner_full));
                        }
                        ("any".to_string(), "any | null".to_string())
                    }
                    other => (other.to_string(), other.to_string()),
                }
            } else {
                ("any".to_string(), "any".to_string())
            }
        }
        _ => ("any".to_string(), "any".to_string()),
    }
}
