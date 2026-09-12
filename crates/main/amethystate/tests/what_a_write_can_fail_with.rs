use amethystate::Store;
use amethystate::amethystate;
use amethystate::errors::WriteValue;
use amethystate::store::builder::StoreBuilder;
use amethystate::store::{KvWrite, field_with_path};
use amethystate_core::test_utils::TempPath;
use std::error::Error;
use uuid::Uuid;

#[amethystate(prefix = "writes")]
pub struct Panel {
    #[amestate(default = 800u32)]
    pub width: u32,
}

fn store(name: &str) -> (TempPath, Store) {
    let at = TempPath::new(name);
    let store = StoreBuilder::new(at.path()).build().unwrap();
    (at, store)
}

fn every_way_a_write_can_fail(why: WriteValue) -> String {
    match why {
        WriteValue::Intercepted { at, said } => format!("{at} was turned down: {said}"),
        WriteValue::Absent { at } => format!("nothing at {at}"),
        WriteValue::NotAPath(said) => format!("no path to land at: {said}"),
        WriteValue::TooDeep { at, why } => format!("{at} is too deep: {}", why.current_context()),
        WriteValue::WillNotEncode { at, why } => {
            format!("{at} will not encode: {}", why.current_context())
        }
        WriteValue::Closed { at } => format!("{at} is behind a closed store"),
        WriteValue::SourceGone => "the cell outlived what it viewed".to_string(),
        WriteValue::Store(why) => format!("the store: {}", why.current_context()),
    }
}

fn every_way_a_raw_write_can_fail(why: KvWrite) -> String {
    match why {
        KvWrite::NotAPath(said) => format!("no path to land at: {said}"),
        KvWrite::Declared { at, by, .. } => format!("{by} declares {at}"),
        KvWrite::TooDeep { at, why } => format!("{at} is too deep: {}", why.current_context()),
        KvWrite::WillNotEncode { at, why } => {
            format!("{at} will not encode: {}", why.current_context())
        }
        KvWrite::Closed { at } => format!("{at} is behind a closed store"),
        KvWrite::Store(why) => format!("the store: {}", why.current_context()),
    }
}

#[test]
fn a_match_over_every_way_a_write_fails_needs_no_catch_all() {
    let (_at, store) = store("writes_exhaustive");

    let panel = Panel::new_with(&store).unwrap();
    let _guard = panel.width().intercept(|_| None);

    let refused = panel.width().set(1024).unwrap_err();

    assert!(
        every_way_a_write_can_fail(refused).contains("writes.width"),
        "the arm that ran must still name the place"
    );
}

#[test]
fn a_match_over_every_way_a_raw_write_fails_needs_no_catch_all() {
    let (_at, store) = store("writes_raw_exhaustive");

    let _panel = Panel::new_with(&store).unwrap();
    let refused = store
        .kv()
        .namespace("writes")
        .set("width", &1u32)
        .unwrap_err();

    assert_eq!(
        every_way_a_raw_write_can_fail(refused),
        "Panel declares writes.width"
    );
}

fn through_anyhow(store: &Store) -> anyhow::Result<()> {
    store.kv().namespace("writes").set("width", &1u32)?;
    Ok(())
}

fn through_a_box(store: &Store) -> Result<(), Box<dyn Error + Send + Sync>> {
    store.kv().namespace("writes").set("width", &1u32)?;
    Ok(())
}

#[test]
fn a_failed_raw_write_goes_into_anyhow_and_into_a_box() {
    let (_at, store) = store("writes_anyhow");

    let carried = through_anyhow(&store).unwrap_err();
    assert!(
        carried.to_string().contains("writes.width"),
        "anyhow keeps what the refusal said, got: {carried}"
    );

    let boxed = through_a_box(&store).unwrap_err();
    assert!(
        boxed.to_string().contains("writes.width"),
        "a boxed error keeps it too, got: {boxed}"
    );
}

fn a_field_write_through_anyhow(store: &Store) -> anyhow::Result<()> {
    let width = field_with_path::<u32>(store, ["loose", "width"], 800, Uuid::new_v4())?;
    width.set(1024)?;
    Ok(())
}

#[test]
fn a_failed_field_write_goes_into_anyhow_too() {
    let (_at, store) = store("writes_field_anyhow");

    store.close().unwrap();

    let carried = a_field_write_through_anyhow(&store).unwrap_err();

    assert!(
        carried.to_string().contains("closed"),
        "anyhow keeps what the refusal said, got: {carried}"
    );
}

#[test]
fn a_write_past_the_cap_says_which_budget_ran_out() {
    let at = TempPath::new("writes_too_deep");
    let shallow = StoreBuilder::new(at.path())
        .limits(|l| l.key_depth(2))
        .build()
        .unwrap();

    let refused = shallow
        .kv()
        .namespace("a")
        .namespace("b")
        .set("c", &1u32)
        .unwrap_err();

    let KvWrite::TooDeep { why, .. } = &refused else {
        panic!("{refused}")
    };

    assert!(
        format!("{why:?}").contains("levels: 3, and the limit is 2"),
        "the report is kept whole, and the numbers are the diagnosis: {why:?}"
    );
}
