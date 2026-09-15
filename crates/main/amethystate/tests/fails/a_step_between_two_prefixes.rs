use amethystate::amethystate;
use amethystate::{AmeData, migrate};

mod v1 {
    use super::*;

    #[amethystate(prefix = "before_move", version = 1)]
    pub struct Moving {
        #[amestate(default = 1u32)]
        pub count: u32,
    }
}

#[amethystate(prefix = "after_move", version = 2)]
pub struct Moving {
    #[amestate(default = 1u32)]
    pub count: u32,
}

#[migrate]
fn migrate_moving_v1_to_v2(old: AmeData<v1::Moving>) -> amethystate::MigrationResult<AmeData<Moving>> {
    Ok(AmeData::<Moving> { count: old.count })
}

fn main() {}
