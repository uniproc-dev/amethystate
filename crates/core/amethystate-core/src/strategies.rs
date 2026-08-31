//! Generators for the names a store has to survive, kept beside the code that
//! decides what a name is.
//!
//! A test asking for a name should not have to know which character separates
//! levels or which one escapes it. Those live here, one file away from the
//! parser that answers for them, so a test says what it wants and a change to
//! the grammar reaches every generator at once.

use crate::path::{ESCAPE, SEPARATOR};
use proptest::prelude::*;

/// One level's name.
///
/// Weighted towards what turns up in a name and towards what breaks one. The
/// letter range is deliberately tiny so different names collide often; digits
/// get their own arm because a map keyed by a number stores exactly those. The
/// punctuation arm is the set that means something to an engine rather than to
/// this library - `*` and `[` to sqlite's `GLOB`, `%` and `_` to `LIKE`, the
/// quotes to every document grammar - and `any::<char>()` alone would sample
/// ten of them out of a million code points.
pub fn segment() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            4 => Just(SEPARATOR),
            4 => Just(ESCAPE),
            6 => prop::char::range('a', 'c'),
            4 => prop::char::range('0', '9'),
            3 => prop_oneof![
                Just('_'), Just('-'), Just(' '), Just('['), Just(']'),
                Just('*'), Just('%'), Just('"'), Just('\''), Just('/'),
            ],
            1 => any::<char>(),
        ],
        1..6,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// A name that holds the separator, with the two halves it was built from, for
/// the tests that have to address both it and the two levels it looks like.
pub fn name_holding_the_separator() -> impl Strategy<Value = (String, String, String)> {
    (segment(), segment())
        .prop_map(|(left, right)| (left.clone(), right.clone(), format!("{left}{SEPARATOR}{right}")))
}

/// A whole stored key rather than one level: what a flat engine hands back,
/// which includes strings this library never wrote. Allowed to be empty, since
/// the root's key is.
pub fn key() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            4 => Just(SEPARATOR),
            4 => Just(ESCAPE),
            4 => prop::char::range('a', 'c'),
            4 => prop::char::range('0', '9'),
            1 => any::<char>(),
        ],
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}
