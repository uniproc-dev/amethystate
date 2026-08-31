#![allow(dead_code)]

use amethystate::ReactiveMap;
use amethystate::migration::fields::AmeStateFields;
use amethystate::migration::types::{AmeType, fnv1a};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// 1. The generic `AmeType` impls in `migration/types.rs`.
//
// These combine a container tag with its parameters by XOR, with nothing
// separating the two. No macro is involved: the collisions below are properties
// of the library itself.
// ---------------------------------------------------------------------------

/// `Option<Option<u32>>` and `u32` carry the same `TYPE_HASH`: `Option`'s tag is
/// XORed in, so two layers cancel each other out.
///
/// A field promoted from `u32` to `Option<Option<u32>>` keeps its `type_hash`,
/// so `calculate_drift` reports no type change and the struct's `SCHEMA_HASH`
/// does not move. Values written as bare integers are then decoded as a nested
/// option, and every read fails or silently defaults.
const _: () = assert!(<Option<Option<u32>> as AmeType>::TYPE_HASH == <u32 as AmeType>::TYPE_HASH);

/// The same cancellation for `Vec`: a field that becomes a matrix looks like a
/// scalar to the migration gate.
const _: () = assert!(<Vec<Vec<u32>> as AmeType>::TYPE_HASH == <u32 as AmeType>::TYPE_HASH);

/// Container nesting order is invisible. `Vec<Option<T>>` (a list that may
/// contain holes) and `Option<Vec<T>>` (a list that may be absent) are entirely
/// different shapes on disk and hash identically.
const _: () =
    assert!(<Vec<Option<u32>> as AmeType>::TYPE_HASH == <Option<Vec<u32>> as AmeType>::TYPE_HASH);

/// A `HashMap` whose key and value types are swapped keeps its `TYPE_HASH`,
/// because `K` and `V` are XORed together.
///
/// Every stored key is parsed with `FromStr` and every value decoded with the
/// backend codec, so after the swap the reader parses `u64` text as a `u32` key
/// and decodes `u32` bytes as a `u64` value. No migration is offered.
const _: () =
    assert!(<HashMap<u32, u64> as AmeType>::TYPE_HASH == <HashMap<u64, u32> as AmeType>::TYPE_HASH);

/// Worse: any `HashMap<T, T>` cancels its parameters entirely, so all of them
/// share one hash - the bare `"HashMap"` tag.
const _: () = assert!(
    <HashMap<u32, u32> as AmeType>::TYPE_HASH == <HashMap<String, String> as AmeType>::TYPE_HASH
);
const _: () = assert!(<HashMap<bool, bool> as AmeType>::TYPE_HASH == fnv1a(b"HashMap"));

// ---------------------------------------------------------------------------
// 2. `#[derive(amethystate::AmeType)]`, which emits `gen_recursive_type_hash`:
//
//     0u32 ^ fnv1a(name_1) ^ H(ty_1) ^ fnv1a(name_2) ^ H(ty_2) ^ ...
//
// This is a plain XOR fold with no seed and no mixing step. It is *not* the
// `schema_hash` fold in `types.rs`; section 3 covers that one.
// ---------------------------------------------------------------------------

#[derive(amethystate::AmeType)]
pub struct SwapBefore {
    pub id: u32,
    pub size: u64,
}

#[derive(amethystate::AmeType)]
pub struct SwapAfter {
    pub id: u64,
    pub size: u32,
}

/// Swapping the types of two fields leaves `TYPE_HASH` untouched, since XOR does
/// not record which name each type was paired with.
///
/// A derived type used as a leaf field is stored as one encoded blob and
/// contributes only this hash to its owner's `FieldDescriptor`, so the owner
/// sees no change either. The blob written by the old build is decoded against
/// the swapped shape with nothing warning that it should not be: `id` reads back
/// the value that was `size` and vice versa, where the widths happen to permit
/// it, and errors out where they do not.
const _: () = assert!(SwapBefore::TYPE_HASH == SwapAfter::TYPE_HASH);

#[derive(amethystate::AmeType)]
pub struct GrowBefore {
    pub anchor: u32,
}

#[derive(amethystate::AmeType)]
pub struct GrowAfter {
    pub anchor: u32,
    pub volume_level: f64,
    pub span_max_len: bool,
}

/// Found by brute force over short field names and the primitive types:
/// `fnv1a("volume_level") ^ H(f64)` equals `fnv1a("span_max_len") ^ H(bool)`, so
/// the two fields cancel and *adding both at once* is free.
///
/// `GrowAfter` gains two fields and reports the same `TYPE_HASH` as
/// `GrowBefore`. As a leaf field the whole struct is one encoded blob, so the
/// change is a real encoding change that no owner's `SCHEMA_HASH` records: the
/// stored blob is short by two members and every load of it fails or defaults,
/// with no drift entry naming the field.
const _: () = assert!(GrowBefore::TYPE_HASH == GrowAfter::TYPE_HASH);

#[derive(amethystate::AmeType)]
pub struct Empty {}

#[derive(amethystate::AmeType)]
pub struct Unit;

#[derive(amethystate::AmeType)]
pub struct TuplePoint(f32, f32);

#[derive(amethystate::AmeType)]
pub enum Mode {
    Idle,
    Busy { retries: u32 },
}

#[derive(amethystate::AmeType)]
pub union RawBits {
    int: u32,
    float: f32,
}

/// Five unrelated shapes all hash to zero.
///
/// The fold seeds at `0u32`, so an empty field list stays zero. `Data::Struct`
/// is the only arm the derive matches, so every enum contributes nothing at
/// all: `Mode` above has a payload field and still hashes to zero, and a union
/// takes the same path. Tuple-struct fields have no `ident`, so
/// `unwrap_or_default()` names them all `""` and any even count of one type
/// cancels.
///
/// Substituting any of these for any other - a marker struct for an enum, an
/// enum for a coordinate pair - is invisible to every consumer of `TYPE_HASH`.
const _: () = assert!(Empty::TYPE_HASH == 0);
const _: () = assert!(Unit::TYPE_HASH == 0);
const _: () = assert!(TuplePoint::TYPE_HASH == 0);
const _: () = assert!(Mode::TYPE_HASH == 0);
const _: () = assert!(RawBits::TYPE_HASH == 0);

/// An enum that gains, loses, or reorders variants keeps hash zero, so a
/// discriminant written by one build is read as a different variant by the next.
#[derive(amethystate::AmeType)]
pub enum ModeExtended {
    Idle,
    Paused,
    Busy { retries: u32 },
}
const _: () = assert!(Mode::TYPE_HASH == ModeExtended::TYPE_HASH);

#[derive(amethystate::AmeType)]
pub struct Meters {
    pub v: f64,
}

#[derive(amethystate::AmeType)]
pub struct Seconds {
    pub v: f64,
}

/// The type's own name is absent from `TYPE_HASH`. The derive documents this as
/// rename tolerance, and it is - but it also means newtypes over one
/// representation are interchangeable, so swapping a field from `Meters` to
/// `Seconds` passes the gate with the numbers reinterpreted in place.
const _: () = assert!(Meters::TYPE_HASH == Seconds::TYPE_HASH);

// ---------------------------------------------------------------------------
// 3. `schema_hash` in `types.rs`, reached as `AmeStateFields::SCHEMA_HASH`.
//
//     h = 0x811c9dc5
//     per field: h ^= fnv1a(name) ^ type_hash; h = h.wrapping_mul(0x01000193)
//
// The multiply between fields makes this fold order-sensitive, so it is not the
// pure XOR of section 2. It is the value `#[migrate]` submits as a step's
// `schema_hash`, and therefore the value the engine compares against stored
// metadata to decide whether a prefix needs work.
// ---------------------------------------------------------------------------

#[amethystate::amethystate(prefix = "swap")]
pub struct FlatSwapV1 {
    #[amestate(default = 0)]
    pub a: u32,
    #[amestate(default = 0)]
    pub b: u64,
}

#[amethystate::amethystate(prefix = "swap")]
pub struct FlatSwapV2 {
    #[amestate(default = 0)]
    pub a: u64,
    #[amestate(default = 0)]
    pub b: u32,
}

/// Positive control. Swapping two field types at the top level *is* caught by
/// `schema_hash`, because the multiply between fields breaks the commutativity
/// that defeats the derive.
const _: () = assert!(
    <FlatSwapV1_Data as AmeStateFields>::SCHEMA_HASH
        != <FlatSwapV2_Data as AmeStateFields>::SCHEMA_HASH
);

/// The same two structs collide under the derive's fold, which is what the
/// generated `TYPE_HASH` uses - and `TYPE_HASH`, not `SCHEMA_HASH`, is what a
/// nested field and the observability `SchemaEntry` carry.
const _: () = assert!(FlatSwapV1_Data::TYPE_HASH == FlatSwapV2_Data::TYPE_HASH);

#[amethystate::amethystate]
pub struct NestedSwapInnerV1 {
    #[amestate(default = 0)]
    pub a: u32,
    #[amestate(default = 0)]
    pub b: u64,
}

#[amethystate::amethystate]
pub struct NestedSwapInnerV2 {
    #[amestate(default = 0)]
    pub a: u64,
    #[amestate(default = 0)]
    pub b: u32,
}

#[amethystate::amethystate(prefix = "nested_swap")]
pub struct NestedSwapOuterV1 {
    #[amestate(nested)]
    pub inner: NestedSwapInnerV1,
}

#[amethystate::amethystate(prefix = "nested_swap")]
pub struct NestedSwapOuterV2 {
    #[amestate(nested)]
    pub inner: NestedSwapInnerV2,
}

/// The derive's blind spot reaches the migration gate through nesting.
///
/// A nested field's `FieldDescriptor` holds `0xDEADBEEF ^ Inner_Data::TYPE_HASH`,
/// and the inner swap does not move `TYPE_HASH`, so the outer `SCHEMA_HASH` -
/// the only hash registered for prefix `nested_swap`, since a nested struct has
/// no prefix of its own - is unchanged. `nested_swap.inner.a` keeps four bytes
/// where the code now expects eight; no step runs and no drift is reported.
const _: () = assert!(
    <NestedSwapOuterV1_Data as AmeStateFields>::SCHEMA_HASH
        == <NestedSwapOuterV2_Data as AmeStateFields>::SCHEMA_HASH
);

#[amethystate::amethystate(prefix = "map_swap")]
pub struct MapSwapV1 {
    pub routes: ReactiveMap<u32, u64>,
}

#[amethystate::amethystate(prefix = "map_swap")]
pub struct MapSwapV2 {
    pub routes: ReactiveMap<u64, u32>,
}

/// `tests/type_hash.rs` proves that changing a `ReactiveMap`'s key type alone,
/// or its value type alone, moves the hash. Changing both at once does not: the
/// map's descriptor is `HashMap<K, V>::TYPE_HASH`, which XORs `K` with `V`.
///
/// Keys are stored as path text and values as encoded bytes, so after the swap
/// every entry is read with the two decoders exchanged.
const _: () = assert!(
    <MapSwapV1_Data as AmeStateFields>::SCHEMA_HASH
        == <MapSwapV2_Data as AmeStateFields>::SCHEMA_HASH
);

#[amethystate::amethystate(prefix = "field_pair")]
pub struct FieldPairV1 {
    #[amestate(default = 0.0)]
    pub volume_level: f64,
}

#[amethystate::amethystate(prefix = "field_pair")]
pub struct FieldPairV2 {
    #[amestate(default = false)]
    pub span_max_len: bool,
}

/// `schema_hash` folds each field as `fnv1a(name) ^ type_hash`, so a field's
/// name and its type are mixed with nothing between them and a change in one can
/// be cancelled by a change in the other. Brute force over short names and the
/// primitives produced this pair.
///
/// These are two entirely different structs with no field in common, and the
/// engine's `target_h != current_h` test cannot tell them apart. The old
/// `field_pair.volume_level` key is never deleted and `field_pair.span_max_len`
/// is never created; the new build reads a missing key and gets `false`.
const _: () = assert!(
    <FieldPairV1_Data as AmeStateFields>::SCHEMA_HASH
        == <FieldPairV2_Data as AmeStateFields>::SCHEMA_HASH
);

/// The same pair collides under the derive's fold too, since a one-field struct
/// reduces both folds to the same per-field term.
const _: () = assert!(FieldPairV1_Data::TYPE_HASH == FieldPairV2_Data::TYPE_HASH);

#[derive(amethystate::AmeType, Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct ZeroTag {
    pub num_max_z: u32,
    pub x_max_zoom: u32,
}

#[amethystate::amethystate(prefix = "gate_off")]
pub struct GateOff {
    #[amestate(default = ZeroTag::default())]
    pub state: ZeroTag,
}

/// Zero is both a reachable `schema_hash` and the engine's sentinel for "no hash
/// recorded", and the field names above were searched for to land on it.
///
/// `component_needs_work` and `migrate_prefix` both guard on `target_hash != 0`,
/// so for this struct every hash comparison is skipped: no drift is ever
/// detected for prefix `gate_off`, whatever its fields later become, and a
/// component whose version has not changed is reported as `Skipped`. The struct
/// has silently opted out of schema checking for the life of the application.
const _: () = assert!(<GateOff_Data as AmeStateFields>::SCHEMA_HASH == 0);

#[amethystate::amethystate(prefix = "two_hashes")]
pub struct TwoHashes {
    #[amestate(default = 0)]
    pub a: u32,
    #[amestate(default = 0)]
    pub b: u64,
}

/// The codebase computes two different numbers for one struct and calls both a
/// schema hash.
///
/// `#[migrate]` registers `AmeStateFields::SCHEMA_HASH` as a step's
/// `schema_hash`, which becomes `PrefixMeta::hash` and the `schema_hash` that
/// `migrate_prefix` writes into `SchemaSnapshot`. The observability
/// `SchemaEntry` registers `_Data::TYPE_HASH` instead, and `ensure_snapshots`
/// compares *that* against the stored snapshot's `schema_hash` to decide whether
/// to rewrite it. The two never agree, so a run that migrates leaves the
/// snapshot holding `SCHEMA_HASH` and `ensure_snapshots` immediately rewrites it
/// with `TYPE_HASH`. Whatever ends up in `SchemaSnapshot::schema_hash` is not the
/// number the migration gate compares, so the field cannot be trusted for
/// diagnostics or reused as a gate later.
const _: () = assert!(TwoHashes_Data::TYPE_HASH != <TwoHashes_Data as AmeStateFields>::SCHEMA_HASH);

#[derive(amethystate::AmeType, Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub enum Shape {
    #[default]
    Circle,
    Square,
}

#[derive(amethystate::AmeType, Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Corner(pub f32, pub f32);

#[amethystate::amethystate(prefix = "zero_typed")]
pub struct ZeroTypedV1 {
    #[amestate(default = Shape::default())]
    pub anchor: Shape,
}

#[amethystate::amethystate(prefix = "zero_typed")]
pub struct ZeroTypedV2 {
    #[amestate(default = Corner::default())]
    pub anchor: Corner,
}

/// Because every enum and every even-arity single-type tuple struct hashes to
/// zero, they are interchangeable as leaf field types all the way up to
/// `SCHEMA_HASH`.
///
/// `anchor` changes from a unit enum to a pair of floats and the migration
/// engine sees an unchanged schema. The stored bytes are the old enum
/// discriminant, which is not a two-element sequence, so the load either errors
/// or drops back to the default on every start with nothing naming the field.
const _: () = assert!(
    <ZeroTypedV1_Data as AmeStateFields>::SCHEMA_HASH
        == <ZeroTypedV2_Data as AmeStateFields>::SCHEMA_HASH
);
