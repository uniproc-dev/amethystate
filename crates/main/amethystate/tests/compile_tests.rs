// Behind `golden`, because trybuild compiles a crate per case and this one test
// costs more than the rest of the suite together.
//
// Every engine is in scope here, and has to be: this crate's dev-dependency on
// itself names them all, so a test target is built with the union whatever the
// command line says. A gate that asked for one engine and not the others asked
// for a build that cannot happen, and the goldens went unchecked.
#[cfg(all(feature = "golden", feature = "redb"))]
#[test]
fn test_macro_expansion_compilation() {
    let t = trybuild::TestCases::new();
    t.pass("tests/expand/basic.rs");
    t.pass("tests/expand/nested.rs");
    t.pass("tests/expand/nested_under_a_dotted_prefix.rs");
    t.pass("tests/expand/map_syntax.rs");
    t.pass("tests/expand/flattened_and_renamed.rs");

    t.compile_fail("tests/fails/subscription_not_clone.rs");
    t.compile_fail("tests/fails/field_loosens_the_struct_rule.rs");
    t.compile_fail("tests/fails/nested_loosens_the_holder_rule.rs");
    t.compile_fail("tests/fails/check_on_a_volatile_field.rs");
    t.compile_fail("tests/fails/check_on_a_nested_field.rs");
    t.compile_fail("tests/fails/a_rule_said_of_the_wrong_kind.rs");

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

    t.compile_fail("tests/fails/serde_on_a_struct_of_paths.rs");
    t.compile_fail("tests/fails/serde_renames_a_field.rs");
    t.compile_fail("tests/fails/a_volatile_field_named_a_place.rs");
    t.compile_fail("tests/fails/amestate_key_is_gone.rs");
    t.compile_fail("tests/fails/flatten_on_a_leaf.rs");
    t.compile_fail("tests/fails/flatten_beside_a_name.rs");
    t.compile_fail("tests/fails/two_flattened_children_meet.rs");
    t.compile_fail("tests/fails/a_flattened_child_meets_a_field.rs");
    t.compile_fail("tests/fails/volatile_and_nested.rs");
    t.compile_fail("tests/fails/volatile_map.rs");
    t.compile_fail("tests/fails/unreadable_rule_misspelt.rs");
    t.compile_fail("tests/fails/wasm_asked_to_persist.rs");
    t.compile_fail("tests/fails/a_modifier_said_twice.rs");
    t.compile_fail("tests/fails/a_field_behind_a_cfg.rs");
}
