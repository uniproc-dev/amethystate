use amethystate::observability::Reason;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{Field, amethystate};
use amethystate_core::test_utils::TempPath;

mod common;

#[amethystate(prefix = "closed_strict")]
pub struct Strict {
    #[amestate(default = 1u8)]
    pub level: u8,
}

#[amethystate(prefix = "closed_lenient", on_unreadable = UseDefault)]
pub struct Lenient {
    #[amestate(default = 1u8)]
    pub level: u8,
}

#[amethystate(prefix = "closed_kept", on_delete = Keep)]
pub struct Kept {
    #[amestate(default = 1u8)]
    pub level: u8,
}

#[amethystate(prefix = "closed_reseeded", on_delete = UseDefault)]
pub struct Reseeded {
    #[amestate(default = 1u8)]
    pub level: u8,
}

#[amethystate(prefix = "closed_mixed", on_unreadable = UseDefault, on_delete = UseDefault)]
pub struct Mixed {
    #[amestate(default = 1u8, on_unreadable = Refuse, on_delete = Keep)]
    pub level: u8,
}

fn settled(field: &Field<u8>, backend: Backend, declared: &str) {
    field.set(7).unwrap();
    assert_eq!(field.get(), 7, "{backend:?} {declared}");
}

fn survives_the_close(field: &Field<u8>, backend: Backend, declared: &str) {
    assert_eq!(
        field.get(),
        7,
        "{backend:?} {declared}: get stopped answering from memory"
    );

    let refused = field
        .try_get()
        .expect_err(&format!("{backend:?} {declared}: try_get kept quiet"));
    assert!(
        matches!(refused.reason, Reason::Closed),
        "{backend:?} {declared}: try_get said {refused:?}"
    );

    assert!(
        field.set(9).is_err(),
        "{backend:?} {declared}: a write was taken"
    );
    assert_eq!(
        field.get(),
        7,
        "{backend:?} {declared}: a refused write moved the value"
    );
}

#[test]
fn every_declared_policy_answers_a_close_the_same_way() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("closed_policies");
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();

        let strict = Strict::new_with(&store).unwrap();
        let lenient = Lenient::new_with(&store).unwrap();
        let kept = Kept::new_with(&store).unwrap();
        let reseeded = Reseeded::new_with(&store).unwrap();
        let mixed = Mixed::new_with(&store).unwrap();

        let declared: [(&str, Field<u8>); 5] = [
            ("refuse/keep", strict.level()),
            ("use-default/keep", lenient.level()),
            ("refuse/keep by name", kept.level()),
            ("refuse/use-default", reseeded.level()),
            ("field tightens the struct", mixed.level()),
        ];

        for (name, field) in &declared {
            settled(field, backend, name);
        }

        store.close().unwrap();

        for (name, field) in &declared {
            survives_the_close(field, backend, name);
        }
    }
}
