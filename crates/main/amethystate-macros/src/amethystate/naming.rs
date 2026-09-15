//! How a field's own name is spelled where it is stored.
//!
//! The source is always a Rust field name, so it is always `snake_case` - which
//! is why every rule here reads words off underscores and never has to guess
//! where one ended.

/// The rules `rename_all` accepts, in the spelling it is written with.
pub(crate) const RULES: &[&str] = &[
    "lowercase",
    "UPPERCASE",
    "PascalCase",
    "camelCase",
    "snake_case",
    "SCREAMING_SNAKE_CASE",
    "kebab-case",
    "SCREAMING-KEBAB-CASE",
];

/// `named` under `rule`, or `None` when the rule is not one of [`RULES`].
pub(crate) fn apply(rule: &str, named: &str) -> Option<String> {
    let words = || named.split('_').filter(|word| !word.is_empty());

    Some(match rule {
        "lowercase" => named.replace('_', "").to_lowercase(),
        "UPPERCASE" => named.replace('_', "").to_uppercase(),
        "snake_case" => named.to_string(),
        "SCREAMING_SNAKE_CASE" => named.to_uppercase(),
        "kebab-case" => named.replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => named.to_uppercase().replace('_', "-"),
        "PascalCase" => words().map(capitalised).collect(),
        "camelCase" => {
            let mut words = words();
            let first = words.next().unwrap_or_default().to_string();
            std::iter::once(first)
                .chain(words.map(capitalised))
                .collect()
        }
        _ => return None,
    })
}

fn capitalised(word: &str) -> String {
    let mut letters = word.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
        None => String::new(),
    }
}
