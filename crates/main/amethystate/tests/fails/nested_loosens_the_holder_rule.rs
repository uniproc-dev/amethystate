use amethystate::amethystate;

#[amethystate(on_unreadable = UseDefault)]
pub struct Db {
    #[amestate(default = 5432u16)]
    pub port: u16,
}

#[amethystate(prefix = "sys", on_unreadable = Refuse)]
pub struct Sys {
    #[amestate(nested)]
    pub db: Db,
}

fn main() {}
