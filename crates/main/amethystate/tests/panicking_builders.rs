//! Where a refused name is reported.
//!
//! The infallible builders panic on a name that cannot be a level, which is
//! only useful if the panic points at the line that wrote the name. Without
//! `#[track_caller]` it points inside the library instead, and the caller is
//! left reading a location in a file they have never opened.

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// The panic hook is process-wide, so two of these at once would read each
/// other's panics.
fn hook_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn panic_location(call: impl FnOnce()) -> (String, u32) {
    let _guard = hook_lock();

    let seen = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);

    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if let Some(at) = info.location() {
            *sink.lock().unwrap() = Some((at.file().to_string(), at.line()));
        }
    }));
    let outcome = panic::catch_unwind(AssertUnwindSafe(call));
    panic::set_hook(previous);

    assert!(outcome.is_err(), "the call was expected to panic");
    let seen = seen.lock().unwrap().take();
    seen.expect("the panic carried no location")
}

fn from_this_file(file: &str) -> bool {
    file.ends_with("panicking_builders.rs")
}

#[test]
fn a_path_built_from_an_empty_level_blames_the_caller() {
    let expected = line!() + 2;
    let (file, line) = panic_location(|| {
        StorePath::from_segments(["ui", ""]);
    });

    assert!(from_this_file(&file), "pointed inside the library: {file}");
    assert_eq!(line, expected);
}

#[test]
fn a_single_empty_level_blames_the_caller() {
    let expected = line!() + 2;
    let (file, line) = panic_location(|| {
        StorePath::segment("");
    });

    assert!(from_this_file(&file), "pointed inside the library: {file}");
    assert_eq!(line, expected);
}

#[test]
fn pushing_an_empty_level_blames_the_caller() {
    let ui = StorePath::segment("ui");

    let expected = line!() + 2;
    let (file, line) = panic_location(move || {
        ui.push("");
    });

    assert!(from_this_file(&file), "pointed inside the library: {file}");
    assert_eq!(line, expected);
}

#[backends(all)]
fn an_empty_namespace_blames_the_caller(backend: Backend) {
    let path = TempPath::new("panicking_namespace");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let expected = line!() + 2;
    let (file, line) = panic_location(move || {
        store.kv().namespace("");
    });

    assert!(from_this_file(&file), "pointed inside the library: {file}");
    assert_eq!(line, expected);
}

/// The fallible forms are the ones that hand the refusal back, and a name out
/// of data goes through those.
#[backends(all)]
fn the_fallible_forms_return_rather_than_panic(backend: Backend) {
    let path = TempPath::new("fallible_namespace");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    assert!(StorePath::try_from_segments(["ui", ""]).is_err());
    assert!(StorePath::try_segment("").is_err());
    assert!(StorePath::segment("ui").try_push("").is_err());
    assert!(store.kv().try_namespace("").is_err());
}
