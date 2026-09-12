use amethystate::amethystate;

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(key = "listen_port", default = 8080u16)]
    pub port: u16,
}

fn main() {}
