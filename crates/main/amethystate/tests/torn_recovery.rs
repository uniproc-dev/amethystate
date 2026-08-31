#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{StoreBackend, StoreLayout};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

mod common;

const PREFIX: &str = "torn";

struct Sidecars {
    data: PathBuf,
    meta: PathBuf,
    data_backup: PathBuf,
    meta_backup: PathBuf,
}

fn sidecars(store: &amethystate::Store) -> Sidecars {
    match StoreBackend::files(store) {
        Some(StoreLayout::Sidecars {
            data,
            meta,
            data_backup,
            meta_backup,
        }) => Sidecars {
            data,
            meta,
            data_backup,
            meta_backup,
        },
        other => panic!("a text store keeps its bookkeeping beside its data, got {other:?}"),
    }
}

fn seeded(backend: Backend, path: &Path, values: [u32; 3]) -> Sidecars {
    let store = StoreBuilder::new(path).backend(backend).build().unwrap();
    let kv = store.kv().namespace(PREFIX);
    kv.set("a", &values[0]).unwrap();
    kv.set("b", &values[1]).unwrap();
    kv.set("c", &values[2]).unwrap();
    store.save_now().unwrap();
    sidecars(&store)
}

fn document_with(backend: Backend, values: [u32; 3]) -> String {
    let path = TempPath::new(&format!("torn_sample_{}", backend.extension()));
    let files = seeded(backend, path.path(), values);
    std::fs::read_to_string(&files.data).unwrap()
}

fn held(store: &amethystate::Store) -> (Option<u32>, Option<u32>, Option<u32>) {
    let kv = store.kv().namespace(PREFIX);
    (
        kv.get::<u32>("a").ok().flatten(),
        kv.get::<u32>("b").ok().flatten(),
        kv.get::<u32>("c").ok().flatten(),
    )
}

fn listing(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    found.sort();
    found
}

fn scratch(name: &str) -> (TempPath, PathBuf) {
    let base = TempPath::new(name);
    let dir = base.path().with_extension("dir");
    std::fs::create_dir_all(&dir).unwrap();
    (base, dir)
}

#[test]
fn a_data_file_that_vanished_is_recovered_from_the_backup_beside_it() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_gone_{}", backend.extension()));
        let files = seeded(backend, path.path(), [11, 22, 33]);

        let good = std::fs::read_to_string(&files.data).unwrap();
        std::fs::write(&files.data_backup, &good).unwrap();
        std::fs::remove_file(&files.data).unwrap();

        let reopened = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap_or_else(|why| {
                panic!(
                    "on {}: a missing data file with a good backup beside it must open: {why:?}",
                    backend.extension()
                )
            });
        let values = held(&reopened);
        let backup_survived = files.data_backup.exists();
        drop(reopened);

        assert_eq!(
            values,
            (Some(11), Some(22), Some(33)),
            "on {}: the data file was gone and the whole store sat in the backup beside it; \
             the store opened empty instead of recovering, and the backup was still there \
             afterwards: {backup_survived}",
            backend.extension()
        );
    }
}

#[test]
fn a_torn_write_with_no_backup_is_refused_at_every_offset() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_offsets_{}", backend.extension()));
        let files = seeded(backend, path.path(), [11, 22, 33]);

        let good = std::fs::read(&files.data).unwrap();
        let good_meta = std::fs::read(&files.meta).unwrap();

        let mut opened_wrong: Vec<(usize, String)> = Vec::new();

        for cut in 1..good.len() {
            std::fs::write(&files.data, &good[..cut]).unwrap();
            std::fs::write(&files.meta, &good_meta).unwrap();
            let _ = std::fs::remove_file(&files.data_backup);
            let _ = std::fs::remove_file(&files.meta_backup);

            let Ok(store) = StoreBuilder::new(path.path()).backend(backend).build() else {
                continue;
            };
            let values = held(&store);
            drop(store);

            if values != (Some(11), Some(22), Some(33)) {
                opened_wrong.push((cut, format!("{values:?}")));
            }
        }

        assert!(
            opened_wrong.is_empty(),
            "on {}: a data file cut short and no backup to repair it from must be refused, \
             not opened with what happens to still parse. Committed values 11/22/33 came \
             back as, by byte offset of the cut: {opened_wrong:?}",
            backend.extension()
        );
    }
}

#[test]
fn a_torn_write_that_still_parses_does_not_eat_the_backup_that_would_repair_it() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_eats_{}", backend.extension()));
        let files = seeded(backend, path.path(), [11, 22, 33]);

        let good = std::fs::read_to_string(&files.data).unwrap();
        let first = good.find("11").expect("the seeded value is in the file");
        let line_end = first + good[first..].find('\n').expect("the file has lines") + 1;

        std::fs::write(&files.data_backup, &good).unwrap();
        std::fs::write(&files.data, &good[..line_end]).unwrap();

        let reopened = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap_or_else(|why| {
                panic!(
                    "on {}: a torn data file with a good backup beside it must open: {why:?}",
                    backend.extension()
                )
            });
        let values = held(&reopened);
        let backup_survived = files.data_backup.exists();
        drop(reopened);
        let left_on_disk = std::fs::read_to_string(&files.data).unwrap();

        assert_eq!(
            values,
            (Some(11), Some(22), Some(33)),
            "on {}: the write was cut off after the first key and the whole document sat in \
             the backup; the stump parsed, so nothing looked broken, the backup was consumed \
             without being read (still there: {backup_survived}) and the stump is now the \
             store: {left_on_disk:?}",
            backend.extension()
        );
    }
}

#[test]
fn a_backup_older_than_the_data_does_not_roll_the_store_back_in_silence() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_stale_{}", backend.extension()));
        let files = seeded(backend, path.path(), [1, 2, 3]);
        let older = std::fs::read_to_string(&files.data).unwrap();

        {
            let store = StoreBuilder::new(path.path()).backend(backend).build().unwrap();
            let kv = store.kv().namespace(PREFIX);
            kv.set("a", &11u32).unwrap();
            kv.set("b", &22u32).unwrap();
            kv.set("c", &33u32).unwrap();
            store.save_now().unwrap();
        }

        std::fs::write(&files.data_backup, &older).unwrap();
        std::fs::write(&files.data, "{ this never finished").unwrap();

        let outcome = StoreBuilder::new(path.path()).backend(backend).build();
        let values = outcome.as_ref().ok().map(held);

        assert_ne!(
            values,
            Some((Some(1), Some(2), Some(3))),
            "on {}: the backup held a state three writes older than the data file it was \
             asked to replace, and the store reported a successful recovery onto it - the \
             committed 11/22/33 is gone and nothing said so",
            backend.extension()
        );
    }
}

#[test]
fn a_backup_a_refused_open_left_behind_is_not_the_truth_at_the_next_one() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_leftover_{}", backend.extension()));
        let files = seeded(backend, path.path(), [1, 2, 3]);
        let good_meta = std::fs::read_to_string(&files.meta).unwrap();

        std::fs::write(&files.meta, "{ this never finished").unwrap();
        assert!(
            StoreBuilder::new(path.path()).backend(backend).build().is_err(),
            "on {}: unreadable bookkeeping with nothing to recover it from must not open",
            backend.extension()
        );
        assert!(
            files.data_backup.exists(),
            "on {}: the refused open left no backup, so there is no leftover to test with",
            backend.extension()
        );

        std::fs::write(&files.data, document_with(backend, [11, 22, 33])).unwrap();
        std::fs::write(&files.meta, &good_meta).unwrap();
        std::fs::write(&files.data, "{ this never finished").unwrap();

        let outcome = StoreBuilder::new(path.path()).backend(backend).build();
        let values = outcome.as_ref().ok().map(held);

        assert_ne!(
            values,
            Some((Some(1), Some(2), Some(3))),
            "on {}: an earlier open was refused and left a backup; the file moved on to \
             11/22/33 afterwards, and when the next write was cut off the store recovered \
             onto that leftover and reported success, three writes in the past",
            backend.extension()
        );
    }
}

#[test]
fn an_open_that_was_refused_leaves_nothing_of_its_own_behind() {
    for backend in common::text_backends() {
        let (base, dir) = scratch(&format!("torn_refused_{}", backend.extension()));
        let store_path = dir.join("settings");
        let files = seeded(backend, &store_path, [11, 22, 33]);

        std::fs::write(&files.meta, "{ this never finished").unwrap();

        let refused = StoreBuilder::new(&store_path).backend(backend).build();
        assert!(
            refused.is_err(),
            "on {}: unreadable bookkeeping with nothing to recover it from must not open",
            backend.extension()
        );

        let found = listing(&dir);
        let expected: Vec<String> = [&files.data, &files.meta]
            .iter()
            .map(|file| file.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        let mut expected = expected;
        expected.sort();

        let _ = std::fs::remove_dir_all(&dir);
        drop(base);

        assert_eq!(
            found, expected,
            "on {}: an open that was refused left a backup beside the store; every later \
             open treats one as an unfinished previous run and recovers onto it",
            backend.extension()
        );
    }
}

#[test]
fn one_buffered_write_does_not_erase_what_another_store_committed() {
    use std::time::Duration;

    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_two_stores_{}", backend.extension()));
        let files = seeded(backend, path.path(), [11, 22, 33]);

        let first = StoreBuilder::new(path.path())
            .backend(backend)
            .disk(|d| d.debounce(Duration::from_secs(60)))
            .build()
            .unwrap();
        first.kv().namespace(PREFIX).set("a", &111u32).unwrap();

        {
            let second = StoreBuilder::new(path.path())
                .backend(backend)
                .disk(|d| d.debounce(Duration::from_secs(60)))
                .build()
                .unwrap();
            second.kv().namespace(PREFIX).set("d", &444u32).unwrap();
            second.save_now().unwrap();
        }

        let committed_elsewhere = std::fs::read_to_string(&files.data).unwrap();
        assert!(
            committed_elsewhere.contains("444"),
            "on {}: the second store's write never reached the file, so there is nothing \
             for the first one to lose",
            backend.extension()
        );

        first.save_now().unwrap();
        drop(first);

        let reopened = StoreBuilder::new(path.path()).backend(backend).build().unwrap();
        let survivor = reopened.kv().namespace(PREFIX).get::<u32>("d").ok().flatten();
        drop(reopened);

        assert_eq!(
            survivor,
            Some(444),
            "on {}: a store holding one unflushed write of its own rewrote the whole \
             document from memory and erased a key another store had already committed - \
             nobody deleted it and the flush reported success",
            backend.extension()
        );
    }
}

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
#[test]
fn a_flush_that_reported_failure_committed_none_of_itself() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    for backend in common::text_backends() {
        let path = TempPath::new(&format!("torn_half_{}", backend.extension()));
        let files = seeded(backend, path.path(), [11, 22, 33]);

        let store = StoreBuilder::new(path.path()).backend(backend).build().unwrap();
        let before = std::fs::read_to_string(&files.data).unwrap();

        const FILE_SHARE_READ: u32 = 1;
        let blocker = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&files.meta)
            .expect("the store's own bookkeeping must be openable");

        store.kv().namespace(PREFIX).set("a", &44u32).unwrap();
        let flushed = store.save_now();
        let after = std::fs::read_to_string(&files.data).unwrap();

        drop(blocker);
        drop(store);

        assert!(
            flushed.is_err(),
            "on {}: bookkeeping that could not be replaced was reported as a flush that landed",
            backend.extension()
        );
        assert_eq!(
            before, after,
            "on {}: the flush returned an error, so the caller has been told nothing landed - \
             and the data file was replaced anyway, leaving new values beside the bookkeeping \
             of the old ones",
            backend.extension()
        );
    }
}

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
const CRASH_CHILD: &str = "AME_TORN_CRASH_CHILD";

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
#[test]
fn a_write_killed_between_the_temporary_and_the_target_leaves_no_temporary_behind() {
    use amethystate::store::config::WriteAttempts;
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::Duration;

    const FILE_SHARE_READ: u32 = 1;
    let backend = common::text_backend();

    if let Ok(child_dir) = std::env::var(CRASH_CHILD) {
        let store_path = PathBuf::from(&child_dir).join("settings");
        let store = StoreBuilder::new(&store_path)
            .backend(backend)
            .file_write(|w| w.replacing(WriteAttempts::times(80).apart(Duration::from_millis(200))))
            .build()
            .unwrap();
        let files = sidecars(&store);
        store.save_now().unwrap();

        let _blocker = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&files.data)
            .expect("the store's own file must be openable");

        store.kv().namespace(PREFIX).set("a", &44u32).unwrap();

        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(700));
            std::process::abort();
        });

        let _ = store.save_now();
        std::process::abort();
    }

    let (base, dir) = scratch("torn_crash");
    let store_path = dir.join("settings");

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("a_write_killed_between_the_temporary_and_the_target_leaves_no_temporary_behind")
        .env(CRASH_CHILD, &dir)
        .output()
        .expect("spawning the writer failed");

    assert!(
        !status.status.success(),
        "the writer was supposed to abort, not exit cleanly"
    );

    let found = listing(&dir);
    let store = StoreBuilder::new(&store_path).backend(backend).build().unwrap();
    let files = sidecars(&store);
    drop(store);

    let mut expected: Vec<String> = [&files.data, &files.meta]
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    expected.sort();

    let _ = std::fs::remove_dir_all(&dir);
    drop(base);

    assert_eq!(
        found, expected,
        "a process killed while its replacement was being retried left the whole document \
         in a temporary file nobody ever collects; one is left per crash, each a full copy \
         of the store"
    );
}
