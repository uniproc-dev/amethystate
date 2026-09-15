use amethystate::amethystate;
use amethystate::migrate;

#[amethystate(prefix = "cfg", version = 2)]
pub struct Cfg {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[migrate]
fn migrate_cfg_v1_to_v2() -> amethystate::MigrationResult<amethystate::AmeData<Cfg>> {
    Ok(amethystate::AmeData::<Cfg> { port: 8080 })
}

fn main() {}
