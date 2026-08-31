use amethystate::amethystate;
use amethystate::store::StorageError;
use amethystate::store::builder::{StoreBuilder, default_backend};
#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
use amethystate::store::config::{FileWritePolicy, WriteAttempts};
#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
use amethystate::store::{StoreBackend, StoreLayout};
use amethystate_core::facts::{StoreFile, all};
use amethystate_core::test_utils::TempPath;
#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
use std::fs::OpenOptions;
#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
use std::time::Instant;

mod common;
use common::{per_engine, shape};

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
fn sidecars(
    store: &amethystate::Store,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    match StoreBackend::files(store) {
        Some(StoreLayout::Sidecars {
            data,
            meta,
            data_backup,
            ..
        }) => (data, meta, data_backup),
        other => panic!("a text store keeps its bookkeeping beside its data, got {other:?}"),
    }
}

#[amethystate(prefix = "atomic")]
pub struct Held {
    #[amestate(default = 1)]
    pub a: u32,

    #[amestate(default = 1)]
    pub b: u32,
}

#[test]
fn a_path_that_cannot_be_written_is_reported() {
    let path = TempPath::new("atomic_unwritable");

    std::fs::create_dir_all(path.path()).unwrap();

    let report = StoreBuilder::new(path.path())
        .build()
        .expect_err("a directory where the store's file goes was opened as a store");

    assert_eq!(
        report.current_context(),
        &StorageError::Open,
        "a path that cannot be used is an open failure, not a read or a codec one"
    );

    let named: Vec<&StoreFile> = all::<StoreFile, _>(&report).collect();
    assert_eq!(
        named,
        vec![&StoreFile(path.path().to_path_buf())],
        "the report must name the file it could not use, as a fact: {report:?}"
    );

    insta::assert_snapshot!(
        per_engine(default_backend(), "open_refused_by_a_directory"),
        shape(&report)
    );
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_write_that_landed_leaves_no_temporary_behind() {
    let base = TempPath::new("atomic_leftover");
    let dir = base.path().with_extension("dir");
    std::fs::create_dir_all(&dir).unwrap();

    let store = StoreBuilder::new(dir.join("settings"))
        .backend(common::text_backend())
        .build()
        .unwrap();
    let held = Held::new_with(&store).unwrap();

    for n in 0..8 {
        held.a().set(n).unwrap();
        store.save_now().unwrap();
    }

    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    found.sort();

    let (data, meta, _) = sidecars(&store);
    let mut expected: Vec<String> = [data, meta]
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    expected.sort();

    drop(store);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        found, expected,
        "eight writes left more beside the store than the store's own two files; \
         the temporary file is meant to become the target, not to accumulate next to it"
    );
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_half_written_file_is_recovered_from_the_backup_beside_it() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("atomic_backup_{}", backend.extension()));

        let data;
        let backup;
        {
            let store = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();
            let held = Held::new_with(&store).unwrap();
            held.a().set(99).unwrap();
            store.save_now().unwrap();

            let (found, _, found_backup) = sidecars(&store);
            data = found;
            backup = found_backup;
        }

        let good = std::fs::read_to_string(&data).unwrap();

        //@show a previous open that never finished
        std::fs::write(&backup, &good).unwrap();
        std::fs::write(&data, "{ this never finished").unwrap();
        //@show-end

        {
            let reopened = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap_or_else(|why| {
                    panic!(
                        "on {}: a half-written file with a good backup beside it must still open: {why:?}",
                        backend.extension()
                    )
                });

            assert_eq!(
                Held::new_with(&reopened).unwrap().a().get(),
                99,
                "on {}: the value from the backup did not reach the reopened store",
                backend.extension()
            );
        }

        assert_eq!(
            std::fs::read_to_string(&data).unwrap(),
            good,
            "on {}: the only good copy was overwritten by the broken file it was there to replace",
            backend.extension()
        );
    }
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_half_written_file_with_no_backup_is_refused() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("atomic_no_backup_{}", backend.extension()));

        let data;
        let backup;
        {
            let store = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();
            let held = Held::new_with(&store).unwrap();
            held.a().set(99).unwrap();
            store.save_now().unwrap();

            let (found, _, found_backup) = sidecars(&store);
            data = found;
            backup = found_backup;
        }

        assert!(
            !backup.exists(),
            "on {}: a store that opened cleanly keeps no backup",
            backend.extension()
        );

        std::fs::write(&data, "{ this never finished").unwrap();

        let refused = StoreBuilder::new(path.path()).backend(backend).build();

        assert!(
            refused.is_err(),
            "on {}: a broken file with nothing to recover from must not open",
            backend.extension()
        );
    }
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn recovery_leaves_the_bookkeeping_agreeing_with_the_data() {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("atomic_meta_{}", backend.extension()));

        let data;
        let meta;
        let backup;
        {
            let store = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();
            let held = Held::new_with(&store).unwrap();
            held.a().set(99).unwrap();
            store.save_now().unwrap();

            let (found, found_meta, found_backup) = sidecars(&store);
            data = found;
            meta = found_meta;
            backup = found_backup;
        }

        let good = std::fs::read_to_string(&data).unwrap();
        let good_meta = std::fs::read_to_string(&meta).unwrap_or_else(|why| {
            panic!(
                "on {}: there is no bookkeeping beside the data to compare: {why}",
                backend.extension()
            )
        });
        assert!(
            !good_meta.trim().is_empty(),
            "on {}: the bookkeeping is empty, so comparing it proves nothing",
            backend.extension()
        );

        std::fs::write(&backup, &good).unwrap();
        std::fs::write(&data, "{ this never finished").unwrap();

        {
            let reopened = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();
            assert_eq!(Held::new_with(&reopened).unwrap().a().get(), 99);
        }

        assert_eq!(
            std::fs::read_to_string(&meta).unwrap(),
            good_meta,
            "on {}: recovery moved the data back and left the bookkeeping somewhere else",
            backend.extension()
        );

        let again = StoreBuilder::new(path.path()).backend(backend).build();
        let again = again.unwrap_or_else(|why| {
            panic!(
                "on {}: a store recovered once must open again: {why:?}",
                backend.extension()
            )
        });

        assert_eq!(
            Held::new_with(&again).unwrap().a().get(),
            99,
            "on {}: the recovered value did not survive a second open",
            backend.extension()
        );
    }
}

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
#[test]
fn a_file_held_by_someone_else_does_not_cost_the_old_contents() {
    use std::os::windows::fs::OpenOptionsExt;

    let path = TempPath::new("atomic_held");

    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .build()
        .unwrap();
    let held = Held::new_with(&store).unwrap();
    held.a().set(5).unwrap();
    store.save_now().unwrap();

    let before = std::fs::read(path.path()).unwrap();

    const FILE_SHARE_READ: u32 = 1;
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path.path())
        .expect("the store's own file must be openable");

    held.a().set(6).unwrap();
    let flushed = store.save_now();

    assert!(
        flushed.is_err(),
        "a write that could not replace the file reported success"
    );
    assert_eq!(
        std::fs::read(path.path()).unwrap_or_default(),
        before,
        "a failed replacement must leave the previous contents where they were"
    );

    drop(blocker);

    store
        .save_now()
        .expect("once the other holder lets go, the same write must land");
    assert_ne!(std::fs::read(path.path()).unwrap(), before);
}

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
#[test]
fn a_holder_that_lets_go_mid_write_does_not_cost_the_write() {
    use std::os::windows::fs::OpenOptionsExt;

    let path = TempPath::new("atomic_midwrite");
    let policy = FileWritePolicy::default();
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .file_write(|_| policy)
        .build()
        .unwrap();
    let held = Held::new_with(&store).unwrap();
    held.a().set(5).unwrap();
    store.save_now().unwrap();

    const FILE_SHARE_READ: u32 = 1;
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path.path())
        .expect("the store's own file must be openable");

    let letting_go = std::thread::spawn(move || {
        std::thread::sleep(policy.replace.budget() / 2);
        drop(blocker);
    });

    held.a().set(6).unwrap();
    let flushed = store.save_now();
    letting_go.join().unwrap();

    flushed.expect("a holder that let go inside the retry budget still cost the write");

    let reopened = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .build()
        .unwrap();
    assert_eq!(
        Held::new_with(&reopened).unwrap().a().get(),
        6,
        "the write reported success without the value reaching the file"
    );
}

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
#[test]
fn a_holder_that_never_lets_go_is_given_up_on() {
    use std::os::windows::fs::OpenOptionsExt;

    let path = TempPath::new("atomic_forever");
    let policy = FileWritePolicy::default();
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .file_write(|_| policy)
        .build()
        .unwrap();
    let held = Held::new_with(&store).unwrap();
    held.a().set(5).unwrap();
    store.save_now().unwrap();

    const FILE_SHARE_READ: u32 = 1;
    let _blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path.path())
        .expect("the store's own file must be openable");

    held.a().set(6).unwrap();
    let started = Instant::now();
    let report = store.save_now().unwrap_err();
    let elapsed = started.elapsed();

    let budget = policy.replace.budget();
    assert!(
        elapsed >= budget,
        "the replacement was given up on after {elapsed:?}, which is less than the \
         configured {budget:?} - a holder letting go a moment later would have cost \
         the write"
    );
    assert!(
        elapsed < budget * 8,
        "a write nobody can complete held the caller for {elapsed:?}"
    );

    let rendered = format!("{report:?}");
    assert!(
        rendered.contains(&path.path().display().to_string()),
        "the failure must name the file it could not replace: {rendered}"
    );
    assert!(
        rendered.contains("os error 5"),
        "the failure must carry what the OS said rather than a summary: {rendered}"
    );
}

#[cfg(all(windows, any(feature = "json", feature = "toml", feature = "ron")))]
#[test]
fn a_policy_that_says_not_to_retry_is_obeyed() {
    use std::os::windows::fs::OpenOptionsExt;

    let path = TempPath::new("atomic_norety");
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .file_write(|w| w.replacing(WriteAttempts::once()))
        .build()
        .unwrap();
    let held = Held::new_with(&store).unwrap();
    held.a().set(5).unwrap();
    store.save_now().unwrap();

    const FILE_SHARE_READ: u32 = 1;
    let _blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path.path())
        .expect("the store's own file must be openable");

    held.a().set(6).unwrap();
    let started = Instant::now();
    assert!(store.save_now().is_err());
    let elapsed = started.elapsed();

    let default_budget = FileWritePolicy::default().replace.budget();
    assert!(
        elapsed < default_budget / 2,
        "asking for no retry still took {elapsed:?}, near the default {default_budget:?} - \
         the configured policy never reached the write path"
    );
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_whole_document_with_bytes_left_after_its_end_is_refused() {
    let path = TempPath::new("atomic_trailing_bytes");

    {
        let store = StoreBuilder::new(path.path())
            .backend(common::text_backend())
            .build()
            .unwrap();
        let held = Held::new_with(&store).unwrap();
        held.a().set(5).unwrap();
        store.save_now().unwrap();
    }

    let good = std::fs::read_to_string(path.path()).unwrap();
    std::fs::write(
        path.path(),
        format!("{good}\n\u{0}\u{0}garbage not in any grammar"),
    )
    .unwrap();

    assert!(
        StoreBuilder::new(path.path())
            .backend(common::text_backend())
            .build()
            .is_err(),
        "the document parsed and what followed it was never looked at, which is \
         how a shorter write over a longer file reads as healthy"
    );
}
