use amethystate_macros::amethystate;

#[amethystate]
pub struct Window {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate(prefix = "editor", rename_all = "camelCase")]
pub struct Editor {
    #[amestate(nested, flatten)]
    pub window: Window,

    #[amestate(default = fourteen())]
    pub font_size: u32,
}

fn fourteen() -> u32 {
    14
}

fn main() {}
