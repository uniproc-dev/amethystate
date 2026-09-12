use amethystate::amethystate;

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(volatile, path = "nowhere", default = 1u32)]
    pub tick: u32,
}

fn main() {}
