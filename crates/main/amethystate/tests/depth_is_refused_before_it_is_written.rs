//! A value the running codec cannot read back is refused at the write.
//!
//! Every codec here reads less deeply than it writes, so the old behaviour was
//! the worst shape a defect can take: the write returned `Ok`, and afterwards
//! the file would not open - or, on redb, the process died and went on dying on
//! every later start, because the value was already committed.
//!
//! The ceilings are the codecs' own, measured rather than chosen, and they are
//! not settings. What is set here is the store's own cap on how deep a *path*
//! may go, and a claim that the contents stay readable on other engines.

#![cfg(any(
    feature = "redb",
    feature = "sqlite",
    feature = "json",
    feature = "toml",
    feature = "ron"
))]

use amethystate::store::WriteValue;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod common;
use common::{once_per_engine, shape};

/// A snapshot name carrying the engine, because the ceiling is the engine's and
/// so the report differs by it.
///
/// `common::per_engine` names after `default_backend`, which is not what these
/// run against - every one of them names its backend.
fn named(label: &str, backend: Backend) -> String {
    format!("{label}_{}", backend.extension())
}

/// Nests exactly `0` deep and no further, so a test can name a number rather
/// than measure one.
#[derive(Clone, Debug, Default, PartialEq)]
struct Ladder(u32);

impl Serialize for Ladder {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        if self.0 == 0 {
            return s.serialize_u32(0);
        }
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(&Ladder(self.0 - 1))?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Ladder {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        u32::deserialize(d).map(Ladder)
    }
}

fn store_at(label: &str, backend: Backend) -> (TempPath, amethystate::Store) {
    let path = TempPath::new(label);
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    (path, store)
}

/// Just inside the ceiling is written; one past it is refused, and the refusal
/// names the number rather than leaving it to be found by experiment.
fn the_ceiling_is_the_codec_s(backend: Backend, label: &str) {
    let ceiling = backend.depth_ceiling();
    let (_path, store) = store_at(label, backend);

    // Two levels of path, so the value may have `ceiling - 2`.
    let at = ["deep", "value"];

    store
        .set(at, &Ladder((ceiling - 2) as u32))
        .expect("a value that exactly fills the budget was refused");

    let report = store
        .set(at, &Ladder((ceiling - 1) as u32))
        .expect_err("a value past the codec's ceiling was taken");

    let WriteValue::WillNotEncode { why: report, .. } = &report else {
        panic!("a value the codec cannot read back is a codec refusal, got {report}")
    };

    // The whole report rather than a phrase out of it. What the refusal has to
    // say - the ceiling, what the path spent of it, and why a deeper value is
    // not merely inconvenient - is the thing being pinned, and a `contains` on
    // any one number would be satisfied by a line number in the same dump.
    insta::assert_snapshot!(named("refuses_past_the_ceiling", backend), shape(report));
}

/// The store keeps working after refusing. A write that was never accepted must
/// not poison what follows it.
fn a_refusal_costs_nothing_else(backend: Backend, label: &str) {
    let (path, store) = store_at(label, backend);
    let ceiling = backend.depth_ceiling();

    store.set(["kept"], &7u32).unwrap();
    let _ = store.set(["too", "deep"], &Ladder(ceiling as u32));

    store.set(["after"], &9u32).unwrap();
    store.save_now().unwrap();
    drop(store);

    let reopened = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .expect("a store that refused a write cannot open its own file");

    assert_eq!(reopened.get::<u32>(["kept"]).unwrap(), Some(7));
    assert_eq!(reopened.get::<u32>(["after"]).unwrap(), Some(9));
    assert_eq!(
        reopened.get::<Ladder>(["too", "deep"]).unwrap(),
        None,
        "a refused value reached the file after all"
    );
}

/// A cap on path levels is the store's own, and is refused where the path is
/// used rather than at some later flush.
fn a_key_depth_cap_is_the_store_s_own(backend: Backend, label: &str) {
    let path = TempPath::new(label);
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .limits(|l| l.key_depth(3))
        .build()
        .unwrap();

    store
        .set(["a", "b", "c"], &1u32)
        .expect("three levels is the cap, not past it");

    let report = store
        .set(["a", "b", "c", "d"], &1u32)
        .expect_err("a path past the store's own cap was accepted");

    let WriteValue::TooDeep { why: report, .. } = &report else {
        panic!("every name is a name a store can hold; there are too many of them: {report}")
    };

    insta::assert_snapshot!(named("refuses_past_the_key_cap", backend), shape(report));

    assert_eq!(
        store.get::<u32>(["a", "b", "c", "d"]).unwrap(),
        None,
        "a refused path was written anyway"
    );
}

/// A store promising to stay readable elsewhere holds to the strictest of the
/// engines it named, not to the one it happens to be running.
#[cfg(all(feature = "redb", feature = "ron"))]
#[test]
fn a_portable_store_holds_to_the_strictest_engine_it_named() {
    let path = TempPath::new("depth_portable");
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Redb)
        .limits(|l| l.portable_across([Backend::Ron]))
        .build()
        .unwrap();

    // Comfortably inside redb's own ceiling of 512, and past ron's 64.
    let report = store
        .set(["p", "v"], &Ladder(100))
        .expect_err("a value ron could not read was written to a store that promised it could");

    // The ceiling in this snapshot is ron's, not the running engine's, which is
    // the whole claim - and a snapshot says so where a `contains` on a number
    // would not distinguish the two.
    let WriteValue::WillNotEncode { why: report, .. } = &report else {
        panic!("{report}")
    };

    insta::assert_snapshot!("refuses_at_the_strictest_named", shape(report));

    let plain = StoreBuilder::new(TempPath::new("depth_unportable").path())
        .backend(Backend::Redb)
        .build()
        .unwrap();
    plain
        .set(["p", "v"], &Ladder(100))
        .expect("the same value is fine on redb alone - the claim is what refused it");
}

once_per_engine! {
    #[test]
    fn the_ceiling_is_the_codecs_own() {
        the_ceiling_is_the_codec_s(BACKEND, &format!("depth_c_{ENGINE}"));
    }

    #[test]
    fn a_refusal_costs_nothing_else() {
        super::a_refusal_costs_nothing_else(BACKEND, &format!("depth_r_{ENGINE}"));
    }

    #[test]
    fn a_key_depth_cap_is_the_stores_own() {
        a_key_depth_cap_is_the_store_s_own(BACKEND, &format!("depth_k_{ENGINE}"));
    }
}
