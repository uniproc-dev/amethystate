use amethystate_macros::amethystate;

#[amethystate(id = "part")]
pub struct Part {
    #[amestate(default = 1280)]
    pub width: u32,
}

fn main() {}
