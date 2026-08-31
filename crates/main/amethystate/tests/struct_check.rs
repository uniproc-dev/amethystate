use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::facts::{Refused, all};
use amethystate::store::{CheckContext, Invalid};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

//@show a check on the struct, for what one field cannot see
fn the_window_can_be_drawn(window: &LenientWindow, _cx: &CheckContext) -> Result<(), Invalid> {
    if window.min().get() <= window.max().get() {
        Ok(())
    } else {
        Err(Invalid::new("the smallest window is wider than the largest")
            .at(&["min", "max"]))
    }
}

#[amethystate(
    prefix = "window_lenient",
    on_unreadable = UseDefault,
    check = the_window_can_be_drawn
)]
pub struct LenientWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,

    #[amestate(default = "amethystate".to_string())]
    pub title: String,
}
//@show-end

fn the_strict_window_can_be_drawn(window: &StrictWindow, _cx: &CheckContext) -> Result<(), Invalid> {
    if window.min().get() <= window.max().get() {
        Ok(())
    } else {
        Err(Invalid::new("the smallest window is wider than the largest"))
    }
}

#[amethystate(prefix = "window_strict", check = the_strict_window_can_be_drawn)]
pub struct StrictWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,
}

fn the_ratio_holds(inner: &Inner, _cx: &CheckContext) -> Result<(), Invalid> {
    if inner.width().get() >= inner.height().get() {
        Ok(())
    } else {
        Err(Invalid::new("a panel taller than it is wide"))
    }
}

#[amethystate(check = the_ratio_holds)]
pub struct Inner {
    #[amestate(default = 800u32)]
    pub width: u32,

    #[amestate(default = 600u32)]
    pub height: u32,
}

#[amethystate(prefix = "holder", on_unreadable = UseDefault)]
pub struct Holder {
    #[amestate(nested)]
    pub panel: Inner,
}

#[backends(all)]
fn a_struct_whose_invariant_fails_does_not_open(backend: Backend) {
    let path = TempPath::new("struct_check_strict");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["window_strict", "min"], &2000u32).unwrap();

    let refused = StrictWindow::new_with(&store).unwrap_err();

    let said: Vec<&Refused> = all::<Refused, _>(&refused).collect();
    assert_eq!(
        said.first().map(|r| r.0.as_str()),
        Some("the smallest window is wider than the largest")
    );
}

#[backends(all)]
fn a_lenient_struct_goes_on_reporting_what_was_stored(backend: Backend) {
    let path = TempPath::new("struct_check_lenient");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["window_lenient", "min"], &2000u32).unwrap();

    let window = LenientWindow::new_with(&store).unwrap();

    assert_eq!(window.min().get(), 2000);
    assert_eq!(window.max().get(), 1600);
}

#[backends(all)]
fn the_complaint_arrives_through_try_get_on_the_named_fields(backend: Backend) {
    let path = TempPath::new("struct_check_named");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["window_lenient", "min"], &2000u32).unwrap();

    let window = LenientWindow::new_with(&store).unwrap();

    assert!(window.min().try_get().is_err());
    assert!(window.max().try_get().is_err());
    assert!(window.title().try_get().is_ok());
}

#[backends(all)]
fn a_refused_relationship_leaves_the_stored_values_where_they_are(backend: Backend) {
    let path = TempPath::new("struct_check_untouched");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["window_lenient", "min"], &2000u32).unwrap();
    let _window = LenientWindow::new_with(&store).unwrap();

    assert_eq!(
        store.get::<u32>(["window_lenient", "min"]).unwrap(),
        Some(2000)
    );
}

#[backends(all)]
fn an_invariant_that_holds_leaves_every_field_alone(backend: Backend) {
    let path = TempPath::new("struct_check_ordinary");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["window_lenient", "min"], &800u32).unwrap();

    let window = LenientWindow::new_with(&store).unwrap();

    assert_eq!(window.min().try_get().unwrap(), 800);
    assert_eq!(window.max().try_get().unwrap(), 1600);
    assert_eq!(window.title().try_get().unwrap(), "amethystate");
}

#[backends(all)]
fn a_nested_struct_is_settled_before_the_one_holding_it_is_built(backend: Backend) {
    let path = TempPath::new("struct_check_nested");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["holder", "panel", "height"], &2000u32).unwrap();

    let holder = Holder::new_with(&store).unwrap();

    assert_eq!(holder.panel().height().get(), 2000);
    assert!(holder.panel().height().try_get().is_err());
    assert!(holder.panel().width().try_get().is_err());
}
