#![cfg(target_arch = "wasm32")]

use amethystate::amethystate;
use amethystate::migration::MigrationError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{
    CheckContext, Invalid, OpenStore, StorageError, StoreLayout, WillNotOpen,
};
use amethystate::{Store, StoreBackend, StoreOp, SubscriptionKind};
use amethystate_core::Source;
use amethystate_core::path::StorePath;
use amethystate_core::primitives::error::WriteValue;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

fn page() -> web_sys::Storage {
    web_sys::window().unwrap().local_storage().unwrap().unwrap()
}

fn opened(name: &str) -> Store {
    StoreBuilder::new(name)
        .backend(Backend::LocalStorage)
        .build()
        .unwrap()
}

fn at(levels: &[&str]) -> StorePath {
    StorePath::from_segments(levels.iter().copied())
}

fn net_steps(
    name: &str,
    step: impl Fn(&mut amethystate::MigrationContext) -> amethystate::MigrationResult<()>
    + Send
    + Sync
    + 'static,
) -> amethystate::store::builder::WithSteps {
    StoreBuilder::new(name)
        .backend(Backend::LocalStorage)
        .migrations(|m| {
            m.for_prefix("net")
                .step(1, "move off the privileged port", step);
        })
}

#[wasm_bindgen_test]
fn a_value_is_kept_as_json_under_the_store_name() {
    let store = opened("page_json");
    store.kv().set("port", &8080u16).unwrap();
    store.kv().set("ui.theme", &"dark").unwrap();
    store.set(at(&["ui", "width"]), &240u16).unwrap();

    assert_eq!(
        store.files_layout(),
        Some(StoreLayout::PageStorage {
            under: at(&["amethystate", "page_json"])
        })
    );
    assert_eq!(
        page().get_item("amethystate.page_json.v.port").unwrap(),
        Some("8080".into())
    );
    assert_eq!(
        page()
            .get_item("amethystate.page_json.v.ui\\.theme")
            .unwrap(),
        Some("\"dark\"".into())
    );
    assert_eq!(
        page().get_item("amethystate.page_json.v.ui.width").unwrap(),
        Some("240".into())
    );
}

#[wasm_bindgen_test]
fn a_store_opened_again_reads_what_the_last_one_wrote() {
    let first = opened("page_reopened");
    first.kv().set("port", &8080u16).unwrap();
    first.set(at(&["ui", "width"]), &240u16).unwrap();
    first.close().unwrap();

    let again = opened("page_reopened");

    assert_eq!(again.kv().get::<u16>("port").unwrap(), Some(8080));
    assert_eq!(again.get::<u16>(&at(&["ui", "width"])).unwrap(), Some(240));
}

#[wasm_bindgen_test]
fn two_stores_in_one_page_do_not_meet() {
    let left = opened("page_left");
    let right = opened("page_right");

    left.kv().set("port", &1u16).unwrap();
    right.kv().set("port", &2u16).unwrap();
    left.delete_prefix(StorePath::root()).unwrap();

    assert_eq!(left.kv().get::<u16>("port").unwrap(), None);
    assert_eq!(right.kv().get::<u16>("port").unwrap(), Some(2));
}

#[wasm_bindgen_test]
fn a_scan_answers_in_path_order_and_only_under_the_prefix() {
    let store = opened("page_scan");
    store.set(at(&["ui", "width"]), &3u16).unwrap();
    store.set(at(&["net", "port"]), &1u16).unwrap();
    store.set(at(&["ui", "height"]), &2u16).unwrap();
    store.set(at(&["uix"]), &9u16).unwrap();

    let found: Vec<String> = store
        .scan_keys(at(&["ui"]))
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();

    assert_eq!(found, ["ui.height", "ui.width"]);
}

#[wasm_bindgen_test]
fn a_write_past_the_quota_is_turned_down_and_the_old_value_stays() {
    let store = opened("page_quota");
    store.kv().set("blob", &"small").unwrap();

    let huge = "x".repeat(12 * 1024 * 1024);
    let WriteValue::Store(refused) = store.set(at(&["blob"]), &huge).unwrap_err() else {
        panic!("the page's refusal should come back as the store's");
    };

    assert_eq!(*refused.current_context(), StorageError::Write);
    assert_eq!(
        store.kv().get::<String>("blob").unwrap(),
        Some("small".into())
    );
}

#[wasm_bindgen_test]
fn a_closed_store_turns_a_read_down() {
    let store = opened("page_closed");
    store.close().unwrap();

    assert_eq!(
        *store.get_raw(&at(&["port"])).unwrap_err().current_context(),
        StorageError::Closed
    );
}

#[wasm_bindgen_test]
fn a_step_runs_over_what_the_page_held() {
    let first = opened("page_migrated");
    first.set(at(&["net", "port"]), &80u16).unwrap();
    first.close().unwrap();

    let (store, report) = net_steps("page_migrated", |ctx| ctx.set("port", &8080u16))
        .migrate()
        .unwrap();

    assert!(!report.has_failures(), "{report:?}");
    assert_eq!(store.get::<u16>(&at(&["net", "port"])).unwrap(), Some(8080));
}

#[wasm_bindgen_test]
fn a_step_that_fails_leaves_the_page_as_it_was() {
    let first = opened("page_unmigrated");
    first.set(at(&["net", "port"]), &80u16).unwrap();
    first.close().unwrap();
    let before = keys_of("amethystate.page_unmigrated.");

    let refused = net_steps("page_unmigrated", |ctx| {
        ctx.set("port", &9090u16)?;
        Err(MigrationError::Custom("this data is not ours".into()).into())
    })
    .migrate();

    assert!(refused.is_err());
    assert_eq!(keys_of("amethystate.page_unmigrated."), before);
    assert_eq!(
        page()
            .get_item("amethystate.page_unmigrated.v.net.port")
            .unwrap(),
        Some("80".into())
    );
}

#[wasm_bindgen_test]
fn a_store_whose_bookkeeping_will_not_read_is_refused_and_left_alone() {
    let first = opened("page_spoiled");
    first.set(at(&["net", "port"]), &80u16).unwrap();
    first.close().unwrap();
    page()
        .set_item("amethystate.page_spoiled.format", "{ not json")
        .unwrap();
    let before = keys_of("amethystate.page_spoiled.");

    let refused = StoreBuilder::new("page_spoiled")
        .backend(Backend::LocalStorage)
        .build()
        .expect_err("opened over bookkeeping that is not JSON");

    assert!(
        matches!(refused, OpenStore::WouldNotOpen { .. }),
        "said {refused}"
    );
    assert_eq!(keys_of("amethystate.page_spoiled."), before);
}

#[wasm_bindgen_test]
fn a_store_told_to_start_fresh_opens_empty_over_bookkeeping_that_will_not_read() {
    let first = opened("page_fresh");
    first.set(at(&["net", "port"]), &80u16).unwrap();
    first.close().unwrap();
    opened("page_fresh_neighbour")
        .set(at(&["net", "port"]), &1u16)
        .unwrap();
    page()
        .set_item("amethystate.page_fresh.meta.net", "{ not json")
        .unwrap();

    let store = StoreBuilder::new("page_fresh")
        .backend(Backend::LocalStorage)
        .when_it_will_not_open(WillNotOpen::StartFresh)
        .build()
        .unwrap();

    assert_eq!(store.get::<u16>(&at(&["net", "port"])).unwrap(), None);
    assert_eq!(
        page()
            .get_item("amethystate.page_fresh_neighbour.v.net.port")
            .unwrap(),
        Some("1".into())
    );
}

#[wasm_bindgen_test]
fn a_store_whose_name_extends_another_is_a_stranger_to_it() {
    let short = opened("page_ns");
    let colon = opened("page_ns:v");
    let dotted = opened("page_ns.v");
    colon.set(at(&["x"]), &1u16).unwrap();
    dotted.set(at(&["x"]), &3u16).unwrap();
    short.set(at(&["format"]), &2u16).unwrap();
    colon.close().unwrap();
    dotted.close().unwrap();

    let seen: Vec<String> = short
        .scan_keys(StorePath::root())
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();

    assert_eq!(seen, ["format"]);
    assert_eq!(
        opened("page_ns:v").get::<u16>(&at(&["x"])).unwrap(),
        Some(1)
    );
    assert_eq!(
        opened("page_ns.v").get::<u16>(&at(&["x"])).unwrap(),
        Some(3)
    );
    assert_eq!(
        page().get_item("amethystate.page_ns:v.v.x").unwrap(),
        Some("1".into())
    );
    assert_eq!(
        page().get_item("amethystate.page_ns\\.v.v.x").unwrap(),
        Some("3".into())
    );
}

#[wasm_bindgen_test]
fn starting_fresh_leaves_a_store_whose_name_extends_it_alone() {
    opened("page_fr:b")
        .set(at(&["net", "port"]), &1u16)
        .unwrap();
    opened("page_fr").set(at(&["net", "port"]), &2u16).unwrap();
    page()
        .set_item("amethystate.page_fr.meta.net", "{ not json")
        .unwrap();

    let _fresh = StoreBuilder::new("page_fr")
        .backend(Backend::LocalStorage)
        .when_it_will_not_open(WillNotOpen::StartFresh)
        .build()
        .unwrap();

    assert_eq!(
        opened("page_fr:b")
            .get::<u16>(&at(&["net", "port"]))
            .unwrap(),
        Some(1)
    );
}

#[wasm_bindgen_test]
fn a_store_asked_for_parallel_reads_reads_a_large_map_on_the_page_thread() {
    let asked = [
        StoreBuilder::in_memory()
            .parallel_reads(true)
            .build()
            .unwrap(),
        StoreBuilder::new("page_parallel")
            .backend(Backend::LocalStorage)
            .parallel_reads(true)
            .build()
            .unwrap(),
    ];

    for store in asked {
        for n in 0..1100u64 {
            store.set(at(&["cols", &format!("c{n:04}")]), &n).unwrap();
        }

        let widths = store.kv().map::<String, u64>("cols").unwrap();

        assert_eq!(widths.len(), 1100);
        assert_eq!(widths.get("c1099"), Some(1099));
    }
}

fn keys_of(prefix: &str) -> Vec<(String, String)> {
    let page = page();
    let mut found: Vec<(String, String)> = (0..page.length().unwrap())
        .filter_map(|at| page.key(at).unwrap())
        .filter(|key| key.starts_with(prefix))
        .map(|key| {
            let value = page.get_item(&key).unwrap().unwrap();
            (key, value)
        })
        .collect();
    found.sort();
    found
}

type Heard = Arc<Mutex<Vec<(StoreOp, String, Option<String>, Source)>>>;

fn listened(store: &Store) -> Heard {
    let heard: Heard = Arc::default();
    let keep = heard.clone();

    StoreBackend::subscribe(
        store,
        SubscriptionKind::Any,
        Arc::new(move |event| {
            keep.lock().unwrap().push((
                event.op,
                event.path.to_string(),
                event
                    .new
                    .as_ref()
                    .map(|bytes| String::from_utf8(bytes.clone()).unwrap()),
                event.source,
            ));
            Ok(())
        }),
    );

    heard
}

fn another_page_wrote(key: &str, old: Option<&str>, new: Option<&str>) {
    let init = web_sys::StorageEventInit::new();
    init.set_key(Some(key));
    init.set_old_value(old);
    init.set_new_value(new);
    init.set_storage_area(Some(&page()));

    let event = web_sys::StorageEvent::new_with_event_init_dict("storage", &init).unwrap();
    web_sys::window()
        .unwrap()
        .unchecked_ref::<web_sys::EventTarget>()
        .dispatch_event(&event)
        .unwrap();
}

#[wasm_bindgen_test]
fn a_change_another_page_made_reaches_a_subscriber_as_one_from_outside() {
    let store = opened("page_other");
    let heard = listened(&store);

    another_page_wrote("amethystate.page_other.v.ui.width", None, Some("320"));
    another_page_wrote("amethystate.page_other.v.ui.width", Some("320"), None);
    another_page_wrote("amethystate.page_elsewhere.v.ui.width", None, Some("1"));
    another_page_wrote("amethystate.page_other.meta.ui", None, Some("{}"));
    another_page_wrote("amethystate.page_other\\.v.v.ui.width", None, Some("2"));

    assert_eq!(
        *heard.lock().unwrap(),
        [
            (
                StoreOp::Set,
                "ui.width".to_string(),
                Some("320".to_string()),
                Source::AnotherPage
            ),
            (
                StoreOp::Delete,
                "ui.width".to_string(),
                None,
                Source::AnotherPage
            ),
        ]
    );
}

fn a_size_that_renders(size: &mut u8, _cx: &CheckContext) -> Result<(), Invalid> {
    match *size >= 6 {
        true => Ok(()),
        false => Err(Invalid::new("a font size below 6 renders nothing")),
    }
}

#[amethystate(prefix = "page_checked", on_unreadable = UseDefault)]
pub struct PageUi {
    #[amestate(default = 14u8, check = a_size_that_renders)]
    pub font_size: u8,
}

#[wasm_bindgen_test]
fn a_change_another_page_made_is_judged_by_the_check() {
    let store = opened("page_checked");
    let ui = PageUi::new_with(&store).unwrap();
    ui.font_size().set(42).unwrap();

    let key = "amethystate.page_checked.v.page_checked.font_size";
    page().set_item(key, "3").unwrap();
    another_page_wrote(key, Some("42"), Some("3"));

    assert_eq!(ui.font_size().get(), 42);
    assert!(ui.font_size().try_get().is_err());
}

#[wasm_bindgen_test]
fn a_closed_store_hears_no_other_page() {
    let store = opened("page_deaf");
    let heard = listened(&store);
    store.close().unwrap();

    another_page_wrote("amethystate.page_deaf.v.ui.width", None, Some("320"));

    assert!(heard.lock().unwrap().is_empty());
}
