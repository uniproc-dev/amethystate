use amethystate::amethystate;
use amethystate::store::{CheckContext, Invalid};

fn a_port_that_is_not_zero(port: &u16, _cx: &CheckContext) -> Result<(), Invalid> {
    if *port == 0 {
        Err(Invalid::new("port 0 asks the machine to pick one"))
    } else {
        Ok(())
    }
}

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(default = 8080u16, volatile, check = a_port_that_is_not_zero)]
    pub port: u16,
}

fn main() {}
