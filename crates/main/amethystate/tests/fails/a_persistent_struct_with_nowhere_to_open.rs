use amethystate::amethystate;

#[amethystate(mode = "persistent")]
pub struct Loose {
    #[amestate(default = 800u32)]
    pub width: u32,
}

fn main() {}
