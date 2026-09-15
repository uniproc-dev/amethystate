use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Window {
    pub width: u32,
    pub height: u32,
}

mod before {
    use super::Window;
    use serde::{Deserialize, Serialize};

    //@show the struct serde was writing
    #[derive(Serialize, Deserialize)]
    pub struct Settings {
        pub theme: String,
        pub window: Window,
    }
    //@show-end
}

//@show the same fields at the top of the store
#[amethystate(mode = "persistent", as_root)]
pub struct Settings {
    #[amestate(default = "light".to_string())]
    pub theme: String,

    #[amestate(default = Window { width: 800, height: 600 })]
    pub window: Window,
}
//@show-end

fn written() -> before::Settings {
    before::Settings {
        theme: "dark".to_string(),
        window: Window {
            width: 1280,
            height: 720,
        },
    }
}

fn in_the_format_of(backend: Backend, settings: &before::Settings) -> anyhow::Result<String> {
    Ok(match backend {
        Backend::Json => serde_json::to_string_pretty(settings)?,
        Backend::Toml => toml_edit::ser::to_string_pretty(settings)?,
        Backend::Ron => ron::ser::to_string_pretty(settings, ron::ser::PrettyConfig::default())?,
        other => unreachable!("{other:?} does not keep a document"),
    })
}

fn read_back(backend: Backend, file: &Path) -> anyhow::Result<before::Settings> {
    let text = std::fs::read_to_string(file)?;

    Ok(match backend {
        Backend::Json => serde_json::from_str(&text)?,
        Backend::Toml => toml_edit::de::from_str(&text)?,
        other => unreachable!("{other:?} does not write a struct back as serde does"),
    })
}

#[backends(Toml)]
fn a_root_struct_opens_the_file_serde_wrote(_backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_custom_as_root");
    let file = path.path();
    std::fs::write(file, in_the_format_of(Backend::Toml, &written())?)?;

    //@show opening the file that was there
    let store = StoreBuilder::new(file).backend(Backend::Toml).build()?;
    let settings = Settings::load_with(&store)?;
    //@show-end

    assert_eq!(settings.theme, "dark");
    assert_eq!(
        settings.window,
        Window {
            width: 1280,
            height: 720
        }
    );

    Ok(())
}

#[backends(Json, Toml)]
fn a_file_taken_over_stays_the_file_serde_reads(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_custom_as_root_text");
    let file = path.path();
    std::fs::write(file, in_the_format_of(backend, &written())?)?;

    let store = StoreBuilder::new(file).backend(backend).build()?;
    let mut settings = Settings::load_with(&store)?;

    assert_eq!(settings.theme, "dark");

    settings.mutate(|now| now.theme = "light".to_string())?;

    let reread = read_back(backend, file)?;

    assert_eq!(reread.theme, "light");
    assert_eq!(
        reread.window,
        Window {
            width: 1280,
            height: 720
        }
    );

    Ok(())
}

#[backends(Ron)]
fn a_ron_file_taken_over_is_written_back_as_the_store_writes_it(
    backend: Backend,
) -> anyhow::Result<()> {
    let path = TempPath::new("book_custom_as_root_ron");
    let file = path.path();
    std::fs::write(file, in_the_format_of(backend, &written())?)?;

    let store = StoreBuilder::new(file).backend(backend).build()?;
    let mut settings = Settings::load_with(&store)?;

    assert_eq!(settings.theme, "dark");

    settings.mutate(|now| now.theme = "light".to_string())?;

    insta::assert_snapshot!(std::fs::read_to_string(file)?);

    Ok(())
}

fn read_old_settings(at: &Path) -> anyhow::Result<Option<before::Settings>> {
    #[derive(Deserialize)]
    struct Appearance {
        theme: String,
    }

    #[derive(Deserialize)]
    struct Legacy {
        appearance: Appearance,
        size: [u32; 2],
    }

    if !at.exists() {
        return Ok(None);
    }

    let legacy: Legacy = serde_json::from_str(&std::fs::read_to_string(at)?)?;

    Ok(Some(before::Settings {
        theme: legacy.appearance.theme,
        window: Window {
            width: legacy.size[0],
            height: legacy.size[1],
        },
    }))
}

#[backends(all)]
fn values_moved_in_once_are_there_on_the_next_open(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_custom_moved_in");
    let old = TempPath::new("book_custom_moved_in_old");
    let file = path.path();
    let old_file = old.path().to_path_buf();

    std::fs::write(
        &old_file,
        r#"{ "appearance": { "theme": "dark" }, "size": [1280, 720] }"#,
    )?;

    {
        let store = StoreBuilder::new(file).backend(backend).build()?;

        //@show moving the values in once
        let mut settings = Settings::load_with(&store)?;

        if let Some(old) = read_old_settings(&old_file)? {
            settings.mutate(|now| {
                now.theme = old.theme;
                now.window = old.window;
            })?;
            std::fs::remove_file(&old_file)?;
        }
        //@show-end
    }

    assert!(!old_file.exists(), "the old file outlived the move");

    let store = StoreBuilder::new(file).backend(backend).build()?;
    let settings = Settings::load_with(&store)?;

    assert_eq!(settings.theme, "dark");
    assert_eq!(
        settings.window,
        Window {
            width: 1280,
            height: 720
        }
    );

    Ok(())
}
