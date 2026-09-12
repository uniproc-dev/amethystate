use amethystate::Store;
use amethystate::amethystate;
use amethystate::Id;
use amethystate::migration::{MigrationError, RunStep};
use amethystate::store::LoadMap;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;
use std::error::Error;

#[amethystate(prefix = "steps", version = 2)]
pub struct Panel {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[derive(Debug)]
pub struct Themes(pub Vec<&'static str>);

fn every_way_a_step_can_fail(why: RunStep) -> String {
    match why {
        RunStep::NothingProvided { wanted, .. } => format!("nobody provided a {wanted}"),
        RunStep::WillNotRead { entry, wanted, .. } => {
            format!("`{entry}` will not read as a {wanted}")
        }
        RunStep::WillNotEncode { entry, wanted, .. } => {
            format!("the {wanted} written to `{entry}` will not encode")
        }
        RunStep::NotAPath(said) => format!("no path to work at: {said}"),
        RunStep::Refused(said) => format!("the step said no: {said}"),
        RunStep::Store(why) => format!("the store: {}", why.current_context()),
    }
}

fn every_way_a_map_can_fail(why: LoadMap) -> String {
    match why {
        LoadMap::NotAPath(said) => format!("no path to sit at: {said}"),
        LoadMap::Taken(taken) => format!("{} already holds it", taken.held_by),
        LoadMap::KeyIsNotAnEntry { stored, .. } => format!("`{stored}` is not an entry"),
        LoadMap::KeyWillNotRead { entry, wanted, .. } => {
            format!("`{entry}` will not read as a {wanted}")
        }
        LoadMap::EntryWillNotRead { at, why } => {
            format!("{at} will not read: {}", why.current_context())
        }
        LoadMap::Closed { under } => format!("{under} is behind a closed store"),
        LoadMap::Store(why) => format!("the store: {}", why.current_context()),
    }
}

fn opened(name: &str) -> (TempPath, Store) {
    let at = TempPath::new(name);
    let store = StoreBuilder::new(at.path()).build().unwrap();
    (at, store)
}

#[test]
fn a_step_that_asks_for_what_nobody_provided_says_so_by_its_variant() {
    let at = TempPath::new("steps_nothing_provided");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let told = seen.clone();

    let _ = StoreBuilder::new(at.path())
        .migrations(move |m| {
            let told = told.clone();
            m.for_node::<Panel>()
                .step(2, "asks for what nobody gave", move |ctx| {
                    let why = ctx.require::<Themes>().unwrap_err();
                    *told.lock().unwrap() = every_way_a_step_can_fail(why);
                    Ok(())
                });
        })
        .build();

    assert_eq!(
        *seen.lock().unwrap(),
        "nobody provided a what_a_step_can_fail_with::Themes"
    );
}

#[test]
fn a_step_reading_the_wrong_shape_says_so_by_its_variant() {
    let at = TempPath::new("steps_will_not_read");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let told = seen.clone();

    {
        let store = StoreBuilder::new(at.path()).build().unwrap();
        store.set(["steps", "width"], &"wide".to_string()).unwrap();
        store.close().unwrap();
    }

    let _ = StoreBuilder::new(at.path())
        .migrations(move |m| {
            let told = told.clone();
            m.for_node::<Panel>()
                .step(2, "reads it as a number", move |ctx| {
                    let why = ctx.get::<u32>("width").unwrap_err();
                    *told.lock().unwrap() = every_way_a_step_can_fail(why);
                    Ok(())
                });
        })
        .build();

    assert_eq!(*seen.lock().unwrap(), "`width` will not read as a u32");
}

#[test]
fn a_step_that_refuses_says_so_by_its_variant() {
    let at = TempPath::new("steps_refused");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let told = seen.clone();

    let _ = StoreBuilder::new(at.path())
        .migrations(move |m| {
            let told = told.clone();
            m.for_node::<Panel>()
                .step(2, "turns the data down", move |_| {
                    let why: RunStep =
                        MigrationError::Custom("this data is not ours".into()).into();
                    *told.lock().unwrap() = every_way_a_step_can_fail(why);
                    Ok(())
                });
        })
        .build();

    assert_eq!(
        *seen.lock().unwrap(),
        "the step said no: Migration error: this data is not ours"
    );
}

#[test]
fn a_match_over_every_way_a_map_load_fails_needs_no_catch_all() {
    let (_at, store) = opened("maps_exhaustive");

    store.set(["cols", "cpu"], &"wide".to_string()).unwrap();

    let refused = store.kv().map::<String, u64>("cols").unwrap_err();

    assert_eq!(
        every_way_a_map_can_fail(refused),
        "cols.cpu will not read: the value could not be encoded or decoded"
    );
}

#[test]
fn a_key_that_is_not_the_maps_key_type_says_so_by_its_variant() {
    let (_at, store) = opened("maps_wrong_key");

    store.set(["ports", "http"], &1u64).unwrap();

    let refused = store.kv().map::<Id<u16>, u64>("ports").unwrap_err();

    assert_eq!(
        every_way_a_map_can_fail(refused),
        "`http` will not read as a Id<u16>"
    );
}

#[test]
fn a_key_deeper_than_an_entry_says_so_by_its_variant() {
    let (_at, store) = opened("maps_too_deep");

    store.set(["widths", "left", "px"], &800u32).unwrap();

    let refused = store.kv().map::<String, u32>("widths").unwrap_err();

    assert_eq!(
        every_way_a_map_can_fail(refused),
        "`widths.left.px` is not an entry"
    );
}

fn through_anyhow(store: &Store) -> anyhow::Result<()> {
    store.set(["ports", "http"], &1u64)?;
    store.kv().map::<Id<u16>, u64>("ports")?;
    Ok(())
}

fn through_a_box(store: &Store) -> Result<(), Box<dyn Error + Send + Sync>> {
    store.set(["ports", "http"], &1u64)?;
    store.kv().map::<Id<u16>, u64>("ports")?;
    Ok(())
}

#[test]
fn a_map_that_will_not_load_goes_into_anyhow_and_into_a_box() {
    let (_at, store) = opened("maps_anyhow");

    let carried = through_anyhow(&store).unwrap_err();
    assert!(
        carried.to_string().contains("http"),
        "anyhow keeps what the refusal said, got: {carried}"
    );

    let boxed = through_a_box(&store).unwrap_err();
    assert!(
        boxed.to_string().contains("http"),
        "a boxed error keeps it too, got: {boxed}"
    );
}
