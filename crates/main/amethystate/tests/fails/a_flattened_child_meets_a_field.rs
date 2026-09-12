use amethystate::amethystate;

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate(prefix = "ui")]
pub struct Ui {
    #[amestate(nested, flatten)]
    pub window: Window,

    #[amestate(default = 0u32)]
    pub width: u32,
}

fn main() {}
