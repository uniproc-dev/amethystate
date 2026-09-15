use amethystate::amethystate;
use amethystate::{AmeData, migrate};

mod v1 {
    use super::*;

    #[amethystate(prefix = "panel", id = "left", version = 1)]
    pub struct Panel {
        #[amestate(default = 1u32)]
        pub width: u32,
    }
}

#[amethystate(prefix = "panel", id = "right", version = 2)]
pub struct Panel {
    #[amestate(default = 1u32)]
    pub width: u32,
}

#[migrate]
fn migrate_panel_v1_to_v2(old: AmeData<v1::Panel>) -> amethystate::MigrationResult<AmeData<Panel>> {
    Ok(AmeData::<Panel> { width: old.width })
}

fn main() {}
