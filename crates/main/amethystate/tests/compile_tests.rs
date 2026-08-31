// One backend, because rustc qualifies type paths differently once others are
// in scope and the .stderr goldens quote it. Behind `golden`, because trybuild
// compiles a crate per case and this one test costs more than the rest of the
// suite together.
#[cfg(all(
    feature = "golden",
    feature = "redb",
    not(feature = "json"),
    not(feature = "toml"),
    not(feature = "ron"),
    not(feature = "sqlite")
))]
#[test]
fn test_macro_expansion_compilation() {
    let t = trybuild::TestCases::new();
    t.pass("tests/expand/basic.rs");
    t.pass("tests/expand/nested.rs");
    t.pass("tests/expand/nested_under_a_dotted_prefix.rs");
    t.pass("tests/expand/map_syntax.rs");

    t.compile_fail("tests/fails/subscription_not_clone.rs");
    t.compile_fail("tests/fails/field_loosens_the_struct_rule.rs");
    t.compile_fail("tests/fails/nested_loosens_the_holder_rule.rs");
    t.compile_fail("tests/fails/check_on_a_volatile_field.rs");
    t.compile_fail("tests/fails/check_on_a_nested_field.rs");

    t.compile_fail("tests/fails/prefix_empty.rs");
    t.compile_fail("tests/fails/prefix_root_dot.rs");
    t.compile_fail("tests/fails/prefix_empty_level.rs");
    t.compile_fail("tests/fails/prefix_trailing_separator.rs");
    t.compile_fail("tests/fails/prefix_holds_the_escape.rs");
    t.compile_fail("tests/fails/key_empty_level.rs");
    t.compile_fail("tests/fails/construction_cycle.rs");
    t.compile_fail("tests/fails/map_through_an_alias.rs");
    t.compile_fail("tests/fails/a_map_by_name_only.rs");
    t.compile_fail("tests/fails/static_path_empty_segment.rs");
    t.compile_fail("tests/fails/static_path_halves_disagree.rs");
}
