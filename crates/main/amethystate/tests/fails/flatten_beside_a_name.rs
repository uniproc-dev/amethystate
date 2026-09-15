use amethystate::amethystate;

#[amethystate]
pub struct Inner {
    #[amestate(default = "localhost".to_string())]
    pub host: String,
}

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(nested, flatten, path = "net")]
    pub inner: Inner,
}

fn main() {}
