use amethystate::amethystate;
use amethystate::store::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::owners::Claimed;
use amethystate_core::facts::all;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;

//@show two structs that want the same place
#[amethystate(prefix = "ui", version = 1)]
pub struct Ui {
    #[amestate(key = "panels.left.visible", default = true)]
    pub left_panel_visible: bool,
}

#[amethystate(prefix = "ui.panels", version = 1)]
pub struct Panels {
    #[amestate(key = "left.visible", default = true)]
    pub left_visible: bool,
}
//@show-end

#[amethystate(prefix = "editor", version = 1)]
pub struct Editor {
    #[amestate(default = 14u32)]
    pub font_size: u32,
}

#[backends(all)]
fn the_second_claim_on_one_place_is_refused(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_claims");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    //@show what the refusal looks like
    let _ui = Ui::new_with(&store)?;

    let refused = Panels::new_with(&store)
        .expect_err("`ui.panels.left.visible` is spelled by both of them");

    assert_eq!(refused.current_context(), &StorageError::Claimed);

    for claim in all::<Claimed, _>(&refused) {
        println!("{} claims {}", claim.by, claim.path);
    }
    //@show-end

    let named: Vec<&str> = all::<Claimed, _>(&refused).map(|claim| claim.by).collect();
    assert_eq!(named.len(), 2, "the report names both: {refused:?}");

    Ok(())
}

#[backends(all)]
fn places_that_do_not_meet_are_left_alone(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_claims_apart");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    //@show two structs that do not meet
    let _ui = Ui::new_with(&store)?;
    let _editor = Editor::new_with(&store)?;
    //@show-end

    Ok(())
}

#[backends(all)]
fn a_claim_outlives_the_handle_that_made_it(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_claims_dropped");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    drop(Ui::new_with(&store)?);

    assert!(
        Panels::new_with(&store).is_err(),
        "dropping the struct must not free the place it claimed"
    );

    Ok(())
}

#[backends(all)]
fn the_store_says_who_claimed_a_place(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_claims_who");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    let _ui = Ui::new_with(&store)?;

    //@show asking who claimed a place
    let field = StorePath::parse_joined("ui.panels.left.visible")?;
    let owner = store.owners().declared_by(&field);

    println!("{owner:?}");
    //@show-end

    assert!(
        owner.is_some_and(|by| by.ends_with("Ui")),
        "the claim is recorded at the field's own path"
    );

    Ok(())
}
