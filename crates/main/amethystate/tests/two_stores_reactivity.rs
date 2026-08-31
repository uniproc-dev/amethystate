//! Whether two stores stay separate at the reactive level.
//!
//! The question behind it: splitting state across files - one per debounce
//! policy, or just one per concern - is only an option if a subscription on one
//! store hears that store and nothing else. The reactive layer is built on
//! `Signal` and `ReactiveMapCore`, which know nothing about storage, so it
//! ought to; this measures it rather than reasoning about it.

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[amethystate(prefix = "fast")]
pub struct Fast {
    #[amestate(default = 0)]
    pub ticks: u32,
}

#[amethystate(prefix = "slow")]
pub struct Slow {
    #[amestate(default = "idle".to_string())]
    pub phase: String,
}

/// A subscriber on one store must not hear the other store's writes, however
/// alike the paths under them look.
#[backends(all)]
fn subscriptions_from_two_stores_are_independent(backend: Backend) {
    let fast_path = TempPath::new("two_stores_indep_fast");
    let slow_path = TempPath::new("two_stores_indep_slow");

    let fast_store = StoreBuilder::new(fast_path.path())
        .backend(backend)
        .build()
        .unwrap();
    let slow_store = StoreBuilder::new(slow_path.path())
        .backend(backend)
        .build()
        .unwrap();

    let fast = Fast::new_with(&fast_store).unwrap();
    let slow = Slow::new_with(&slow_store).unwrap();

    let fast_hits = Arc::new(AtomicUsize::new(0));
    let slow_hits = Arc::new(AtomicUsize::new(0));

    let f = fast_hits.clone();
    let _a = fast.ticks().subscribe(move |_| {
        f.fetch_add(1, Ordering::SeqCst);
    });
    let s = slow_hits.clone();
    let _b = slow.phase().subscribe(move |_| {
        s.fetch_add(1, Ordering::SeqCst);
    });

    fast.ticks().set(1).unwrap();
    fast.ticks().set(2).unwrap();
    slow.phase().set("busy".to_string()).unwrap();

    assert_eq!(fast_hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        slow_hits.load(Ordering::SeqCst),
        1,
        "a subscriber in one store did not hear the other store's writes"
    );
}
