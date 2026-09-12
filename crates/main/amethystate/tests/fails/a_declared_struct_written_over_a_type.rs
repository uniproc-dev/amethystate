use amethystate::amethystate;

#[amethystate(prefix = "cfg")]
pub struct Cfg<T> {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(volatile, default = None)]
    pub held: Option<T>,
}

fn main() {}
