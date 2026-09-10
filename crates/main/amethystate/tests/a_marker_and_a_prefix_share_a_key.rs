#![cfg(feature = "bench-internals")]

use amethystate::store::StorePath;
use amethystate::store::backend::utils::init_key;

#[test]
#[ignore = "known: a marker and a declared prefix share one key space, so `prefix = \"init.foo\"` \
            lands on the marker for `foo` - see TODO.md"]
fn a_declared_prefix_cannot_be_mistaken_for_an_initialisation_marker() {
    let marker = init_key(&StorePath::segment("foo"));
    let declared = StorePath::from_segments(["init", "foo"]).key();

    assert_ne!(
        marker.as_bytes(),
        declared.as_bytes(),
        "a component declared at `init.foo` writes its bookkeeping where the marker \
         for the namespace `foo` lives, so one overwrites the other"
    );
}

#[test]
#[ignore = "known: the same collision, one level deeper - see TODO.md"]
fn the_marker_for_a_nested_namespace_is_its_own_key() {
    let marker = init_key(&StorePath::from_segments(["ui", "panels"]));
    let declared = StorePath::from_segments(["init", "ui", "panels"]).key();

    assert_ne!(marker.as_bytes(), declared.as_bytes());
}
