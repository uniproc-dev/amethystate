use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{StoreBackend, StoreLayout};
use amethystate_test_macros::backends;
use std::path::{Path, PathBuf};

/// A name nothing else on this machine would choose.
const APP: &str = "amethystate-relocation-probe";

struct Litter(PathBuf);

impl Litter {
    /// The probe's own directory and nothing above it.
    ///
    /// `remove_dir_all` is what runs on this, so the directory is found by name
    /// rather than by counting levels upward, and a file that turns out not to
    /// be under one panics instead of widening the sweep.
    fn around(file: &Path) -> Self {
        let dir = file
            .ancestors()
            .find(|dir| named_after_the_probe(dir))
            .unwrap_or_else(|| panic!("{} is not under {APP}", file.display()));

        Self(dir.to_path_buf())
    }
}

impl Drop for Litter {
    fn drop(&mut self) {
        if named_after_the_probe(&self.0) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn named_after_the_probe(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(APP))
}

//@show moving every file a store is made of
fn relocate(from: &StoreLayout, to: &StoreLayout) -> std::io::Result<()> {
    for (old, new) in from.names().iter().zip(to.names()) {
        if old.exists() {
            std::fs::rename(old, new)?;
        }
    }

    Ok(())
}
//@show-end

fn dir_of(layout: &StoreLayout) -> PathBuf {
    layout
        .names()
        .first()
        .and_then(|file| file.parent().map(Path::to_path_buf))
        .expect("the configuration directory the platform answered with")
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

#[backends(all)]
fn a_store_is_found_where_an_older_release_left_it(backend: Backend) {
    moved(backend).unwrap();
}

fn moved(backend: Backend) -> anyhow::Result<()> {
    let app = format!("{APP}-{}", backend.extension());

    let before = StoreBuilder::located(|at| at.app(&app, "settings-legacy"))?
        .backend(backend)
        .build()?;

    let was =
        StoreBackend::files_layout(&before).expect("every engine in this crate names its files");
    let _litter = Litter::around(&was.names()[0]);

    before.set(["ui", "width"], &800u32)?;
    before.close()?;

    //@show a store found where an older release left it
    let store = StoreBuilder::located(|at| {
        let now = at.app(&app, "settings")?;
        let was = at.files_at(at.app(&app, "settings-legacy")?, backend);

        relocate(&was, &at.files_at(&now, backend)).ok();

        Ok(now)
    })?
    .backend(backend)
    .build()?;
    //@show-end

    assert_eq!(
        store.get::<u32>(["ui", "width"])?,
        Some(800),
        "the value came with the store"
    );

    assert!(
        was.present().is_empty(),
        "nothing is left where the store used to be: {:?}",
        was.present()
    );

    let now = StoreBackend::files_layout(&store).expect("the store names its files");
    store.close()?;

    let mut named = now.present();
    named.sort();

    assert_eq!(
        entries(&dir_of(&now)),
        named,
        "a closed store is exactly the files its layout names",
    );

    Ok(())
}
