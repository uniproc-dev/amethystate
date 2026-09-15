use amethystate::amethystate;

#[derive(Clone, Default)]
pub struct Opaque(u64);

mod as_a_number {
    use super::Opaque;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Opaque, into: S) -> Result<S::Ok, S::Error> {
        into.serialize_u64(value.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(from: D) -> Result<Opaque, D::Error> {
        u64::deserialize(from).map(Opaque)
    }
}

#[amethystate(prefix = "opaque")]
pub struct Holder {
    #[amestate(with = as_a_number)]
    pub value: Opaque,
}

fn main() {}
