use amethystate::amethystate;

#[amethystate(prefix = "cfg", on_unreadable = Refuse)]
pub struct Cfg {
    #[amestate(default = 0u16, on_unreadable = UseDefault)]
    pub port: u16,
}

fn main() {}
