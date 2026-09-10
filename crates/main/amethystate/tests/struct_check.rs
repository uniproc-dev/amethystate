use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{CheckContext, Invalid, OpenStruct};
use amethystate::{AmeData, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

//@show a check on the struct, for what one field cannot see
fn the_window_can_be_drawn(
    window: &AmeData<LenientWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new("the smallest window is wider than the largest").at(&["min", "max"]))
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

fn the_strict_window_can_be_drawn(
    window: &AmeData<StrictWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new(
            "the smallest window is wider than the largest",
        ))
    }
}

#[amethystate(prefix = "window_strict", check = the_strict_window_can_be_drawn)]
pub struct StrictWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,
}

fn the_ratio_holds(inner: &AmeData<Inner>, _cx: &CheckContext) -> Result<(), Invalid> {
    if inner.width >= inner.height {
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

    match StrictWindow::new_with(&store).unwrap_err() {
        OpenStruct::Refused { at, said } => {
            assert_eq!(at.to_string(), "window_strict");
            assert_eq!(&*said, "the smallest window is wider than the largest");
        }
        other => panic!("{other}"),
    }
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

//@show the same rule, on a struct that is loaded rather than watched
#[amethystate(prefix = "kept_window", mode = "persistent", check = the_kept_window_can_be_drawn)]
pub struct KeptWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,
}

fn the_kept_window_can_be_drawn(
    window: &AmeData<KeptWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new(
            "the smallest window is wider than the largest",
        ))
    }
}
//@show-end

#[backends(all)]
fn a_loaded_struct_whose_invariant_fails_does_not_load(backend: Backend) {
    let path = TempPath::new("struct_check_kept_strict");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["kept_window", "min"], &2000u32).unwrap();

    match KeptWindow::load_with(&store) {
        Ok(_) => panic!("a window whose min is past its max loaded"),
        Err(OpenStruct::Refused { at, said }) => {
            assert_eq!(at.to_string(), "kept_window");
            assert_eq!(&*said, "the smallest window is wider than the largest");
        }
        Err(other) => panic!("{other}"),
    }
}

#[backends(all)]
fn a_loaded_struct_whose_invariant_holds_loads_what_was_stored(backend: Backend) {
    let path = TempPath::new("struct_check_kept_ok");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["kept_window", "min"], &500u32).unwrap();

    let kept = KeptWindow::load_with(&store).unwrap();

    assert_eq!(kept.min, 500);
    assert_eq!(kept.max, 1600);
}

#[amethystate(
    prefix = "kept_lenient",
    mode = "persistent",
    on_unreadable = UseDefault,
    check = the_lenient_kept_window_can_be_drawn
)]
pub struct LenientKeptWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,
}

fn the_lenient_kept_window_can_be_drawn(
    window: &AmeData<LenientKeptWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new(
            "the smallest window is wider than the largest",
        ))
    }
}

#[backends(all)]
fn a_lenient_loaded_struct_keeps_what_was_stored(backend: Backend) {
    let path = TempPath::new("struct_check_kept_lenient");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["kept_lenient", "min"], &2000u32).unwrap();

    let kept = LenientKeptWindow::load_with(&store).unwrap();

    assert_eq!(kept.min, 2000);
    assert_eq!(kept.max, 1600);
}

#[amethystate(prefix = "either_window", mode = "both", check = the_either_window_can_be_drawn)]
pub struct EitherWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,
}

fn the_either_window_can_be_drawn(
    window: &AmeData<EitherWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new(
            "the smallest window is wider than the largest",
        ))
    }
}

#[backends(all)]
fn one_check_serves_both_constructors(backend: Backend) {
    let path = TempPath::new("struct_check_either");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["either_window", "min"], &2000u32).unwrap();

    assert!(EitherWindow::new_with(&store).is_err());
    assert!(EitherWindow::load_with(&store).is_err());
}

fn the_renamed_window_can_be_drawn(
    window: &AmeData<RenamedWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min_width <= window.max_width {
        Ok(())
    } else {
        Err(
            Invalid::new("the smallest window is wider than the largest")
                .at(&["min_width", "max_width"]),
        )
    }
}

#[amethystate(
    prefix = "window_renamed",
    rename_all = "camelCase",
    on_unreadable = UseDefault,
    check = the_renamed_window_can_be_drawn
)]
pub struct RenamedWindow {
    #[amestate(default = 400u32)]
    pub min_width: u32,

    #[amestate(default = 1600u32)]
    pub max_width: u32,
}

#[backends(all)]
fn a_complaint_reaches_a_field_stored_under_another_name(backend: Backend) {
    let path = TempPath::new("struct_check_renamed");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["window_renamed", "minWidth"], &2000u32).unwrap();

    let window = RenamedWindow::new_with(&store).unwrap();

    assert!(
        window.min_width().try_get().is_err(),
        "the check named `min_width`, and the field is stored as `minWidth`"
    );
    assert!(window.max_width().try_get().is_err());
}
