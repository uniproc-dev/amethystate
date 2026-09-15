//! What the compiler says a declared field is.
//!
//! The role and the optionality in `FIELDS` are read off the type by
//! `shape::Probe`, not off how the type was written. So the interesting cases
//! are the ones where the two disagree: a map reached through an alias is still
//! a map, and a type alias for a plain value is still not optional.

use amethystate::amethystate;
use amethystate::migration::fields::{AmeStateFields, Role};
use amethystate::reactive::map::ReactiveMap;
use amethystate::shape::{AnyShape as _, Probe};

type Aliased = ReactiveMap<String, u64>;
type Port = u16;
type Maybe = Option<String>;

/// A type this crate has never heard of, standing in for one from a crate where
/// no derive could be added even if we wanted one.
struct Foreign;

/// The probe answers at compile time, so this is where the answers are checked.
/// A file that compiles is a set of claims that held.
#[allow(clippy::assertions_on_constants)]
const _: () = {
    assert!(<Probe<Foreign>>::ROLE.same(Role::Field));
    assert!(!<Probe<Foreign>>::OPTIONAL);

    assert!(
        <Probe<Option<Foreign>>>::OPTIONAL,
        "the modifier is visible even though the type inside it implements nothing"
    );

    assert!(
        <Probe<Aliased>>::ROLE.same(Role::Map),
        "an aliased map is a map"
    );
    assert!(
        !<Probe<Port>>::OPTIONAL,
        "an alias for a plain value is not optional"
    );
};

#[amethystate(prefix = "shape")]
pub struct Shaped {
    #[amestate(default = 8080)]
    pub port: Port,

    #[amestate(default = None)]
    pub note: Maybe,

    #[amestate(default = {})]
    pub widths: ReactiveMap<String, u64>,
}

fn field(name: &str) -> &'static amethystate::migration::fields::FieldDescriptor {
    <Shaped_Data as AmeStateFields>::FIELDS
        .iter()
        .find(|f| f.name.as_str() == name)
        .unwrap_or_else(|| panic!("no field named {name}"))
}

/// And what the probe answered is what the store's record of the shape holds.
#[test]
fn the_descriptors_carry_what_the_compiler_answered() {
    assert_eq!(field("port").role, Role::Field);
    assert!(!field("port").optional);

    assert_eq!(field("note").role, Role::Field);
    assert!(
        field("note").optional,
        "`Maybe` is `Option<String>`, and the compiler sees through the alias"
    );

    assert_eq!(field("widths").role, Role::Map);
    assert!(!field("widths").optional);
}
