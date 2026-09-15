use amethystate_macros::amethystate;

#[amethystate(prefix = "editor")]
pub struct Editor {
    #[amestate(volatile, nested)]
    pub window: Window,
}

#[amethystate]
pub struct Window {
    #[amestate(default = 1280)]
    pub width: u32,
}

fn main() {}
