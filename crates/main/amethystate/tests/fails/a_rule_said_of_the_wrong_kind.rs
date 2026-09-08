use amethystate::amethystate;

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(default = {}, on_unreadable = UseDefault)]
    pub widths: amethystate::ReactiveMap<String, u64>,

    #[amestate(default = 1u64, unreadable_entries = Skip)]
    pub width: u64,
}

fn main() {}
