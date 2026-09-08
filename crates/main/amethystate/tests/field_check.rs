use amethystate::amethystate;
use amethystate::observability::Reason;
use amethystate::store::builder::StoreBuilder;
use amethystate::store::{CheckContext, Invalid, OpenStruct};
use amethystate_core::test_utils::TempPath;

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
mod common;

pub struct InstalledThemes(pub Vec<&'static str>);

//@show a check on a field, and the world it is judged against
fn a_size_that_renders(size: &u8, _cx: &CheckContext) -> Result<(), Invalid> {
    if *size >= 6 {
        Ok(())
    } else {
        Err(Invalid::new("a font size below 6 renders nothing"))
    }
}

fn a_theme_that_is_installed(theme: &String, cx: &CheckContext) -> Result<(), Invalid> {
    let installed = cx.require::<InstalledThemes>()?;

    if installed.0.contains(&theme.as_str()) {
        Ok(())
    } else {
        Err(Invalid::new(format!(
            "no theme called {theme} is installed"
        )))
    }
}

#[amethystate(prefix = "checked_lenient", on_unreadable = UseDefault)]
pub struct LenientUi {
    #[amestate(default = 14u8, check = a_size_that_renders)]
    pub font_size: u8,

    #[amestate(default = "dark".to_string(), check = a_theme_that_is_installed)]
    pub theme: String,
}
//@show-end

#[amethystate(prefix = "checked_strict")]
pub struct StrictUi {
    #[amestate(default = 14u8, check = a_size_that_renders)]
    pub font_size: u8,
}

#[amethystate(prefix = "checked_loaded", mode = "persistent", on_unreadable = UseDefault)]
pub struct LoadedUi {
    #[amestate(default = 14u8, check = a_size_that_renders)]
    pub font_size: u8,
}

#[amethystate(prefix = "checked_loaded_strict", mode = "persistent")]
pub struct StrictLoadedUi {
    #[amestate(default = 14u8, check = a_size_that_renders)]
    pub font_size: u8,
}

fn themes() -> InstalledThemes {
    InstalledThemes(vec!["dark", "solarized"])
}

#[test]
fn a_value_the_check_refuses_does_not_open_a_strict_struct() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_strict");
    let store = StoreBuilder::new(path.path()).build()?;

    store.set(["checked_strict", "font_size"], &3u8)?;

    match StrictUi::new_with(&store).unwrap_err() {
        OpenStruct::Refused { said, .. } => {
            assert_eq!(&*said, "a font size below 6 renders nothing")
        }
        other => panic!("{other}"),
    }

    Ok(())
}

#[test]
fn a_refused_open_hands_over_the_path_and_the_reason() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_matched");
    let store = StoreBuilder::new(path.path()).build()?;

    store.set(["checked_strict", "font_size"], &3u8)?;

    //@show telling one refused open from another
    match StrictUi::new_with(&store) {
        Ok(_) => {}
        Err(OpenStruct::Refused { at, said }) => eprintln!("{at} will not do: {said}"),
        Err(OpenStruct::WillNotRead { at, why }) => eprintln!("{at} is unreadable: {why}"),
        Err(OpenStruct::Taken(taken)) => {
            eprintln!("{} already holds {}", taken.held_by, taken.at)
        }
        Err(other) => return Err(other.into()),
    }
    //@show-end

    Ok(())
}

#[test]
fn try_get_hands_over_the_same_facts_the_open_would_have() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_try_get_matched");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    store.set(["checked_lenient", "font_size"], &3u8)?;

    let ui = LenientUi::new_with(&store)?;

    //@show asking a field what the store disagrees with
    let held = match ui.font_size().try_get() {
        Ok(size) => size,
        Err(no) => {
            match no.reason {
                Reason::Refused(said) => eprintln!("running on the default: {said}"),
                Reason::WillNotRead(said) => eprintln!("{} will not decode: {said}", no.at),
                Reason::Occupied(said) => eprintln!("{} was already taken: {said}", no.at),
                _ => eprintln!("{} is not what the store has", no.at),
            }

            ui.font_size().get()
        }
    };
    //@show-end

    assert_eq!(held, 14);

    Ok(())
}

#[test]
fn a_refused_value_takes_the_default_and_try_get_says_why() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_lenient");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    store.set(["checked_lenient", "font_size"], &3u8)?;

    let ui = LenientUi::new_with(&store)?;

    assert_eq!(ui.font_size().get(), 14);
    assert!(ui.font_size().try_get().is_err());
    assert!(ui.theme().try_get().is_ok());

    Ok(())
}

#[test]
fn a_refused_value_is_left_where_it_is() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_untouched");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    store.set(["checked_lenient", "font_size"], &3u8)?;
    let _ui = LenientUi::new_with(&store)?;

    assert_eq!(store.get::<u8>(["checked_lenient", "font_size"])?, Some(3));

    Ok(())
}

#[test]
fn a_value_the_check_accepts_is_read_as_it_is() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_accepted");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    store.set(["checked_lenient", "font_size"], &20u8)?;

    let ui = LenientUi::new_with(&store)?;

    assert_eq!(ui.font_size().try_get()?, 20);

    Ok(())
}

#[test]
fn a_check_judges_the_value_against_what_the_application_gave() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_context");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    store.set(["checked_lenient", "theme"], &"solarized".to_string())?;

    let ui = LenientUi::new_with(&store)?;

    assert_eq!(ui.theme().try_get()?, "solarized");

    Ok(())
}

#[test]
fn a_theme_the_application_does_not_have_is_refused() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_unknown_theme");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    store.set(["checked_lenient", "theme"], &"midnight".to_string())?;

    let ui = LenientUi::new_with(&store)?;

    assert_eq!(ui.theme().get(), "dark");
    assert!(ui.theme().try_get().is_err());

    Ok(())
}

#[test]
fn a_check_whose_input_nobody_gave_refuses_the_value() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_no_context");
    let store = StoreBuilder::new(path.path()).build()?;

    store.set(["checked_lenient", "theme"], &"solarized".to_string())?;

    let ui = LenientUi::new_with(&store)?;

    assert_eq!(ui.theme().get(), "dark");
    assert!(ui.theme().try_get().is_err());

    Ok(())
}

#[test]
fn a_loaded_struct_takes_the_default_over_a_refused_value() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_loaded");
    let store = StoreBuilder::new(path.path()).build()?;

    store.set(["checked_loaded", "font_size"], &3u8)?;

    let ui = LoadedUi::load_with(&store)?;

    assert_eq!(ui.font_size, 14);

    Ok(())
}

#[test]
fn a_strict_loaded_struct_refuses_to_load_at_all() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_loaded_strict");
    let store = StoreBuilder::new(path.path()).build()?;

    store.set(["checked_loaded_strict", "font_size"], &3u8)?;

    match StrictLoadedUi::load_with(&store) {
        Ok(_) => panic!("a value the check refuses must not load"),
        Err(OpenStruct::Refused { said, .. }) => {
            assert_eq!(&*said, "a font size below 6 renders nothing")
        }
        Err(other) => panic!("{other}"),
    }

    Ok(())
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn an_edit_from_outside_that_the_check_refuses_is_not_taken() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_external");
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .context(themes())
        .build()?;

    let ui = LenientUi::new_with(&store)?;
    ui.font_size().set(42)?;
    store.save_now()?;

    let on_disk = std::fs::read_to_string(path.path())?;
    std::fs::write(path.path(), on_disk.replace("42", "3"))?;

    store.set(["elsewhere", "poke"], &1u8)?;

    assert_eq!(ui.font_size().get(), 42);
    assert!(ui.font_size().try_get().is_err());
    assert_eq!(store.get::<u8>(["checked_lenient", "font_size"])?, Some(3));

    Ok(())
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn an_edit_from_outside_the_check_accepts_arrives() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_external_good");
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .context(themes())
        .build()?;

    let ui = LenientUi::new_with(&store)?;
    ui.font_size().set(42)?;
    store.save_now()?;

    let on_disk = std::fs::read_to_string(path.path())?;
    std::fs::write(path.path(), on_disk.replace("42", "18"))?;

    store.set(["elsewhere", "poke"], &1u8)?;

    assert_eq!(ui.font_size().try_get()?, 18);

    Ok(())
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_value_a_second_store_committed_is_judged_the_same_as_a_hand_edit() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_second_store");
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .context(themes())
        .build()?;

    let ui = LenientUi::new_with(&store)?;
    ui.font_size().set(42)?;
    store.save_now()?;

    {
        let other = StoreBuilder::new(path.path())
            .backend(common::text_backend())
            .context(themes())
            .build()?;
        other.set(["checked_lenient", "font_size"], &3u8)?;
        other.save_now()?;
    }

    store.set(["elsewhere", "poke"], &1u8)?;

    assert_eq!(ui.font_size().get(), 42);
    assert!(ui.font_size().try_get().is_err());

    Ok(())
}

#[test]
fn a_store_that_agrees_with_every_check_opens_quietly() -> anyhow::Result<()> {
    let path = TempPath::new("field_check_ordinary");
    let store = StoreBuilder::new(path.path()).context(themes()).build()?;

    let ui = LenientUi::new_with(&store)?;

    assert_eq!(ui.font_size().try_get()?, 14);
    assert_eq!(ui.theme().try_get()?, "dark");

    Ok(())
}
