use amethystate::store::builder::StoreBuilder;
use amethystate::{InitGlobal, StoreBackend, global_store};
use amethystate_core::test_utils::TempPath;
#[cfg(not(target_arch = "wasm32"))]
use serial_test::serial;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

fn port_in(path: &TempPath) -> Option<u16> {
    StoreBuilder::new(path.path())
        .build()
        .unwrap()
        .kv()
        .get::<u16>("port")
        .unwrap()
}

#[test]
#[cfg_attr(not(target_arch = "wasm32"), serial(global))]
fn a_closed_global_store_makes_room_for_the_next() {
    let first = TempPath::new("global_closed_first");
    let second = TempPath::new("global_closed_second");

    StoreBuilder::new(first.path())
        .build_global()
        .unwrap()
        .close()
        .unwrap();

    let guard = StoreBuilder::new(second.path()).build_global().unwrap();
    global_store().kv().set("port", &9090u16).unwrap();
    guard.close().unwrap();

    assert_eq!(port_in(&second), Some(9090));
    assert_eq!(port_in(&first), None);
}

#[test]
#[cfg_attr(not(target_arch = "wasm32"), serial(global))]
fn a_dropped_guard_makes_room_for_the_next() {
    let first = TempPath::new("global_dropped_first");
    let second = TempPath::new("global_dropped_second");

    drop(StoreBuilder::new(first.path()).build_global().unwrap());

    let guard = StoreBuilder::new(second.path()).build_global().unwrap();
    assert!(!global_store().is_closed());
    guard.close().unwrap();
}

#[test]
#[cfg_attr(not(target_arch = "wasm32"), serial(global))]
fn an_awaited_close_makes_room_for_the_next() {
    let first = TempPath::new("global_awaited_first");
    let second = TempPath::new("global_awaited_second");

    let guard = StoreBuilder::new(first.path()).build_global().unwrap();
    futures::executor::block_on(guard.close_async()).unwrap();

    let (guard, _report) = StoreBuilder::new(second.path()).migrate_global().unwrap();
    assert!(!global_store().is_closed());
    guard.close().unwrap();
}

#[test]
#[cfg_attr(not(target_arch = "wasm32"), serial(global))]
fn a_global_store_still_open_refuses_a_second() {
    let first = TempPath::new("global_open_first");
    let second = TempPath::new("global_open_second");

    let guard = StoreBuilder::new(first.path()).build_global().unwrap();

    let refused = StoreBuilder::new(second.path())
        .build_global()
        .expect_err("a second global store was put in place over an open one");
    assert!(matches!(refused, InitGlobal::AlreadyInstalled));

    guard.close().unwrap();
}

#[test]
#[cfg_attr(not(target_arch = "wasm32"), serial(global))]
fn a_handle_kept_from_a_closed_global_store_stays_closed() {
    let first = TempPath::new("global_kept_first");
    let second = TempPath::new("global_kept_second");

    let guard = StoreBuilder::new(first.path()).build_global().unwrap();
    let kept = global_store();
    guard.close().unwrap();

    let next = StoreBuilder::new(second.path()).build_global().unwrap();

    assert!(kept.is_closed());
    kept.kv()
        .set("port", &8080u16)
        .expect_err("a handle to the closed store took a write");
    assert!(!global_store().is_closed());

    next.close().unwrap();
}

#[test]
#[cfg_attr(not(target_arch = "wasm32"), serial(global))]
fn an_old_guard_dropped_late_leaves_the_next_store_open() {
    let first = TempPath::new("global_late_first");
    let second = TempPath::new("global_late_second");

    let old = StoreBuilder::new(first.path()).build_global().unwrap();
    amethystate::shutdown().unwrap();

    let next = StoreBuilder::new(second.path()).build_global().unwrap();
    drop(old);

    assert!(!global_store().is_closed());
    global_store().kv().set("port", &9090u16).unwrap();
    next.close().unwrap();

    assert_eq!(port_in(&second), Some(9090));
}
