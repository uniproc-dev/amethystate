use amethystate::migration::MigrationContext;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "shelf", id = "a_reader", version = 1)]
    pub struct Reader {
        pub seen: u32,
    }

    #[amethystate(prefix = "shelf", id = "b_source", version = 1)]
    pub struct Source {
        pub count: u32,
    }
}

#[amethystate(prefix = "shelf", id = "a_reader", version = 2)]
pub struct Reader {
    pub seen: u32,
    pub from_source: u32,
}

#[amethystate(prefix = "shelf", id = "b_source", version = 2)]
pub struct Source {
    pub count: u32,
    pub doubled: u32,
}

#[migrate]
fn migrate_reader_v1_to_v2(
    old: AmeData<v1::Reader>,
    ctx: &mut MigrationContext,
) -> amethystate::MigrationResult<AmeData<Reader>> {
    Ok(AmeData::<Reader> {
        seen: old.seen,
        from_source: ctx.global_get::<u32>("shelf.doubled")?.unwrap_or(0),
    })
}

#[migrate]
fn migrate_source_v1_to_v2(
    old: AmeData<v1::Source>,
) -> amethystate::MigrationResult<AmeData<Source>> {
    Ok(AmeData::<Source> {
        count: old.count,
        doubled: old.count * 2,
    })
}

#[backends(all)]
fn a_step_reading_its_neighbours_key_sees_it_migrated(backend: Backend) {
    let at = TempPath::new("shelf");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        v1::Reader::new_with(&store).unwrap().seen().set(1).unwrap();
        v1::Source::new_with(&store)
            .unwrap()
            .count()
            .set(7)
            .unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .migrate()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    assert_eq!(
        Reader::new_with(&store).unwrap().from_source().get(),
        14,
        "{backend:?}"
    );
}
