#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;

mod common;

#[test]
fn a_deep_undeclared_path_is_one_key_and_reads_back() {
    let mut wrong = Vec::new();

    for backend in common::text_backends() {
        let ceiling = backend.depth_ceiling();

        for over in [1usize, 8, 64, 512] {
            let levels = ceiling + over;
            let path = TempPath::new(&format!("past_ceiling_{}_{over}", backend.extension()));

            let names: Vec<String> = (0..levels).map(|n| format!("n{n}")).collect();
            let at = StorePath::from_segments(&names);

            {
                let store = StoreBuilder::new(path.path())
                    .backend(backend)
                    .build()
                    .unwrap();
                store.set(&at, &7u32).unwrap_or_else(|why| {
                    panic!(
                        "{}: {levels} levels refused, and nothing declares them so they \
                         are one key: {why:?}",
                        backend.extension()
                    )
                });
                store.save_now().unwrap();
            }

            let text = std::fs::read_to_string(path.path()).unwrap();
            let opened = text.matches('{').count().max(text.matches('[').count());
            if opened > 2 {
                wrong.push(format!(
                    "{}: a path of {levels} levels nested the document {opened} deep, \
                     which is past the {ceiling} this engine reads",
                    backend.extension()
                ));
            }

            let store = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();
            let read = store.get::<u32>(&at).unwrap();
            if read != Some(7) {
                wrong.push(format!(
                    "{}: {levels} levels came back as {read:?}",
                    backend.extension()
                ));
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
