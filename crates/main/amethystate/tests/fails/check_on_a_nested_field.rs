use amethystate::amethystate;
use amethystate::store::{CheckContext, Invalid};

fn a_reachable_host(inner: &Inner, _cx: &CheckContext) -> Result<(), Invalid> {
    if inner.host().get().is_empty() {
        Err(Invalid::new("a host nobody can reach"))
    } else {
        Ok(())
    }
}

#[amethystate]
pub struct Inner {
    #[amestate(default = "localhost".to_string())]
    pub host: String,
}

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(nested, check = a_reachable_host)]
    pub net: Inner,
}

fn main() {}
