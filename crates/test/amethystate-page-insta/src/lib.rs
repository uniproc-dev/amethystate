//! insta, wherever the suite runs.
//!
//! Off the web this is insta itself. In a page it is insta's two assertions
//! and nothing else: a page has no disk, so insta cannot read a snapshot
//! there. The test target's snapshots are compiled into it, found by the name
//! insta would give them, and compared the way insta compares them. A
//! snapshot that does not exist yet is printed for a person to write down:
//! nothing in a page can write it.

use std::cell::RefCell;
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
pub use insta::*;

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use amethystate_test_macros::snapshots as __snapshots;

/// What the snapshot is called: the name the caller gave, or the function the
/// assertion sits in.
#[doc(hidden)]
pub enum Name {
    Given(String),
    Function(&'static str),
}

#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! assert_snapshot {
    ($name:expr, $value:expr $(,)?) => {
        $crate::check(
            $crate::__snapshots!(),
            ::core::module_path!(),
            $crate::Name::Given(::std::string::ToString::to_string(&$name)),
            &::std::string::ToString::to_string(&$value),
        )
    };
    ($value:expr $(,)?) => {
        $crate::check(
            $crate::__snapshots!(),
            ::core::module_path!(),
            $crate::Name::Function($crate::__function_name!()),
            &::std::string::ToString::to_string(&$value),
        )
    };
}

#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! assert_debug_snapshot {
    ($name:expr, $value:expr $(,)?) => {
        $crate::check(
            $crate::__snapshots!(),
            ::core::module_path!(),
            $crate::Name::Given(::std::string::ToString::to_string(&$name)),
            &::std::format!("{:#?}", $value),
        )
    };
    ($value:expr $(,)?) => {
        $crate::check(
            $crate::__snapshots!(),
            ::core::module_path!(),
            $crate::Name::Function($crate::__function_name!()),
            &::std::format!("{:#?}", $value),
        )
    };
}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
#[macro_export]
macro_rules! __function_name {
    () => {{
        fn f() {}
        let mut name = ::std::any::type_name_of_val(&f)
            .strip_suffix("::f")
            .unwrap_or("");
        while let Some(rest) = name.strip_suffix("::{{closure}}") {
            name = rest;
        }
        name
    }};
}

thread_local! {
    static SEEN: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

/// The stem insta gives a snapshot: the module path, then the name.
fn stem(module: &str, name: Name) -> String {
    let module = module.replace("::", "__");

    let name = match name {
        Name::Given(name) => name,
        Name::Function(function) => {
            let bare = function.rsplit("::").next().unwrap_or(function);
            let bare = bare.strip_prefix("test_").unwrap_or(bare);
            let key = format!("{module}::{bare}");
            let nth = SEEN.with(|seen| {
                let mut seen = seen.borrow_mut();
                let nth = seen.entry(key).or_default();
                *nth += 1;
                *nth
            });
            match nth {
                1 => bare.to_string(),
                nth => format!("{bare}-{nth}"),
            }
        }
    };

    format!("{module}__{name}")
}

/// What a `.snap` file holds below its header.
fn body(file: &str) -> &str {
    let file = file.strip_prefix('\u{feff}').unwrap_or(file);
    let mut rest = file;
    let mut fences = 0;

    while let Some(end) = rest.find('\n') {
        let line = rest[..end].trim_end();
        rest = &rest[end + 1..];
        if line == "---" {
            fences += 1;
            if fences == 2 {
                return rest;
            }
        }
    }

    file
}

fn normalised(text: &str) -> String {
    text.trim_end().replace("\r\n", "\n")
}

#[doc(hidden)]
#[track_caller]
pub fn check(stored: &[(&str, &str)], module: &str, name: Name, actual: &str) {
    let stem = stem(module, name);

    let Some((_, file)) = stored.iter().find(|(held, _)| *held == stem) else {
        panic!(
            "no snapshot `{stem}`: nothing in a page can write one, so write \
             tests/snapshots/{stem}.snap by hand, holding\n{actual}"
        );
    };

    let expected = normalised(body(file));
    let actual = normalised(actual);
    let legacy = expected.trim_start_matches(['\r', '\n']);

    if expected != actual && legacy != actual {
        panic!("snapshot `{stem}` does not match\n--- stored\n{expected}\n--- now\n{actual}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_given_name_follows_the_module_it_was_asserted_in() {
        assert_eq!(stem("kv", Name::Given("kv_write".into())), "kv__kv_write");
        assert_eq!(
            stem("kv::localstorage", Name::Given("kv_write".into())),
            "kv__localstorage__kv_write"
        );
    }

    #[test]
    fn a_function_name_drops_its_test_prefix_and_counts_repeats() {
        let name = || Name::Function("field_check::test_a_context_is_listed");

        assert_eq!(
            stem("field_check", name()),
            "field_check__a_context_is_listed"
        );
        assert_eq!(
            stem("field_check", name()),
            "field_check__a_context_is_listed-2"
        );
    }

    #[test]
    fn the_body_is_what_follows_the_header() {
        let file = "---\r\nsource: tests/kv.rs\r\nexpression: err\r\n---\r\nthe store said no\r\n";

        assert_eq!(normalised(body(file)), "the store said no");
    }

    #[test]
    fn a_stored_snapshot_that_matches_passes() {
        let stored = [("kv__said", "---\nsource: x\n---\nline one\nline two\n")];

        check(
            &stored,
            "kv",
            Name::Given("said".into()),
            "line one\nline two",
        );
    }

    #[test]
    #[should_panic(expected = "does not match")]
    fn a_stored_snapshot_that_differs_fails() {
        let stored = [("kv__said", "---\nsource: x\n---\nline one\n")];

        check(&stored, "kv", Name::Given("said".into()), "line two");
    }

    #[test]
    #[should_panic(expected = "write tests/snapshots/kv__missing.snap")]
    fn a_missing_snapshot_says_what_to_write() {
        check(&[], "kv", Name::Given("missing".into()), "anything");
    }
}
