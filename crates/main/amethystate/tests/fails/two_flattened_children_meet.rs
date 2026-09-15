use amethystate::amethystate;

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate]
pub struct Sidebar {
    #[amestate(default = 200u32)]
    pub width: u32,
}

#[amethystate(prefix = "ui")]
pub struct Ui {
    #[amestate(nested, flatten)]
    pub window: Window,

    #[amestate(nested, flatten)]
    pub sidebar: Sidebar,
}

fn main() {}
