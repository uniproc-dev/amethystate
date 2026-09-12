use amethystate::store::builder::StoreBuilder;
use amethystate::store::{OpenStruct, field_with_path};
use amethystate::{Store, amethystate};
use amethystate_core::test_utils::TempPath;
use std::error::Error;
use uuid::Uuid;

#[amethystate(prefix = "boundary")]
pub struct Panel {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate(prefix = "boundary.width")]
pub struct Overlapping {
    #[amestate(default = 1u32)]
    pub px: u32,
}

fn store(name: &str) -> (TempPath, Store) {
    let at = TempPath::new(name);
    let store = StoreBuilder::new(at.path()).build().unwrap();
    (at, store)
}

fn every_way_it_can_fail(why: OpenStruct) -> String {
    match why {
        OpenStruct::Refused { at, said } => format!("{at} was refused: {said}"),
        OpenStruct::WillNotRead { at, why } => {
            format!("{at} will not read: {}", why.current_context())
        }
        OpenStruct::Taken(taken) => format!(
            "{} wants {}, {} holds {}",
            taken.wanted_by, taken.at, taken.held_by, taken.held_at
        ),
        OpenStruct::NotAPath(said) => format!("no path to sit at: {said}"),
        OpenStruct::Store(said) => format!("the store: {}", said.current_context()),
    }
}

#[test]
fn a_match_over_every_way_an_open_fails_needs_no_catch_all() {
    let (_at, store) = store("boundary_exhaustive");

    Panel::new_with(&store).unwrap();
    let refused = Overlapping::new_with(&store).unwrap_err();

    assert!(
        every_way_it_can_fail(refused).contains("boundary.width"),
        "the arm that ran must still name the place"
    );
}

fn through_anyhow(store: &Store) -> anyhow::Result<Overlapping> {
    Ok(Overlapping::new_with(store)?)
}

#[test]
fn a_failed_open_goes_into_anyhow_with_a_question_mark() {
    let (_at, store) = store("boundary_anyhow");

    Panel::new_with(&store).unwrap();
    let carried = through_anyhow(&store).unwrap_err();

    assert!(
        carried.to_string().contains("boundary.width"),
        "anyhow keeps what the refusal said, got: {carried}"
    );
}

fn through_a_box(store: &Store) -> Result<Overlapping, Box<dyn Error + Send + Sync>> {
    Ok(Overlapping::new_with(store)?)
}

#[test]
fn a_failed_open_goes_into_a_boxed_error_with_a_question_mark() {
    let (_at, store) = store("boundary_boxed");

    Panel::new_with(&store).unwrap();
    let carried = through_a_box(&store).unwrap_err();

    assert!(
        carried.to_string().contains("boundary.width"),
        "a boxed error keeps what the refusal said, got: {carried}"
    );
}

#[test]
fn levels_that_do_not_make_a_path_are_their_own_answer() {
    let (_at, store) = store("boundary_not_a_path");

    let nothing: Vec<String> = Vec::new();
    let why = field_with_path(&store, nothing, 1u32, Uuid::new_v4()).unwrap_err();

    assert!(
        matches!(why, OpenStruct::NotAPath(_)),
        "a list of no levels is not the store's fault: {why}"
    );
    assert!(why.source().is_some(), "and it names what it came from");
}

#[test]
fn a_refusal_whose_source_is_the_store_still_hands_the_report_over() {
    let (_at, store) = store("boundary_store_source");

    Panel::new_with(&store).unwrap();
    let refused = Overlapping::new_with(&store).unwrap_err();

    assert!(
        refused.source().is_none(),
        "a claim is the library's own answer, with nothing underneath it"
    );
}
