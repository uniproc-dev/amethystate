use amethystate::amethystate;

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(default = 8080u16, on_unreadble = UseDefault)]
    pub port: u16,

    #[amestate(default = 1u8, volatil)]
    pub retries: u8,
}

fn main() {}
