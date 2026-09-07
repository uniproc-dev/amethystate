use amethystate::amethystate;
use amethystate::store::OpenStruct;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::places::Taken;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

//@show two structs that want the same place
#[amethystate(prefix = "ui", version = 1)]
pub struct Ui {
    #[amestate(path = "panels.left.visible", default = true)]
    pub left_panel_visible: bool,
}

#[amethystate(prefix = "ui.panels", version = 1)]
pub struct Panels {
    #[amestate(path = "left.visible", default = true)]
    pub left_visible: bool,
}
//@show-end

//@show a struct that sits right beside one and still opens
#[amethystate(prefix = "ui.panels.right", version = 1)]
pub struct RightPanel {
    #[amestate(default = true)]
    pub visible: bool,
}
//@show-end

#[backends(all)]
fn the_second_claim_on_one_place_is_refused(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_claims");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    //@show what the refusal looks like
    let _ui = Ui::new_with(&store)?;

    let refused =
        Panels::new_with(&store).expect_err("`ui.panels.left.visible` is spelled by both of them");

    let OpenStruct::Taken(taken) = &refused else {
        panic!("{refused}")
    };

    let Taken {
        at,
        wanted_by,
        held_at,
        held_by,
    } = &**taken;

    println!("{wanted_by} wants {at}, which {held_by} already holds at {held_at}");
    //@show-end

    assert!(wanted_by.ends_with("Panels"));
    assert!(held_by.ends_with("Ui"));
    assert_eq!(at.as_str(), "ui.panels.left.visible");
    assert_eq!(held_at.as_str(), "ui.panels.left.visible");

    Ok(())
}

#[backends(all)]
fn places_that_do_not_meet_are_left_alone(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_claims_apart");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    let _ui = Ui::new_with(&store)?;
    let _right = RightPanel::new_with(&store)?;

    Ok(())
}

#[backends(all)]
fn a_claim_outlives_the_handle_that_made_it(backend: Backend) -> anyhow::Result<()> {
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
fn the_store_says_who_claimed_a_place(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_claims_who");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

    let _ui = Ui::new_with(&store)?;

    //@show asking who claimed a place
    let field = StorePath::parse_joined("ui.panels.left.visible")?;
    let owner = store.places().declared_by(&field);

    println!("{owner:?}");
    //@show-end

    assert!(
        owner.is_some_and(|by| by.ends_with("Ui")),
        "the claim is recorded at the field's own path"
    );

    Ok(())
}
