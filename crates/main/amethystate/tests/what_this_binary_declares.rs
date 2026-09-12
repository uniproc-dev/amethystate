use amethystate::amethystate;
use amethystate::schema::{declarations, declarations_at};
use amethystate_core::path::StorePath;

#[amethystate(prefix = "reader.one", version = 3)]
pub struct One {
    #[amestate(default = 1u8)]
    pub a: u8,
}

#[amethystate(prefix = "reader.shared", version = 1)]
pub struct Beside {
    #[amestate(default = 1u8)]
    pub a: u8,
}

#[amethystate(prefix = "reader.shared", version = 1)]
pub struct AndBeside {
    #[amestate(default = 1u8)]
    pub b: u8,
}

fn names_at(prefix: &str) -> Vec<&'static str> {
    let at = StorePath::parse_joined(prefix).unwrap();
    let mut found: Vec<&'static str> = declarations_at(&at)
        .map(|entry| entry.struct_name)
        .collect();
    found.sort();
    found
}

#[test]
fn every_declaration_is_in_the_list_the_reader_hands_over() {
    let names: Vec<&str> = declarations().iter().map(|e| e.struct_name).collect();

    for declared in ["One", "Beside", "AndBeside"] {
        assert!(names.contains(&declared), "{declared} is not in {names:?}");
    }
}

#[test]
fn asking_twice_is_the_same_list() {
    assert!(std::ptr::eq(declarations(), declarations()));
}

#[test]
fn two_declarations_at_one_prefix_both_come_back() {
    assert_eq!(names_at("reader.shared"), ["AndBeside", "Beside"]);
    assert_eq!(names_at("reader.one"), ["One"]);
    assert_eq!(names_at("reader"), Vec::<&str>::new());
}

#[test]
fn a_declaration_carries_what_it_declared() {
    let at = StorePath::parse_joined("reader.one").unwrap();
    let one = declarations_at(&at).next().expect("One is declared");

    assert_eq!(one.version, 3);
    assert_eq!(one.fields.len(), 1);
}
