use amethystate::amethystate;

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(flatten, default = "localhost".to_string())]
    pub host: String,
}

fn main() {}
