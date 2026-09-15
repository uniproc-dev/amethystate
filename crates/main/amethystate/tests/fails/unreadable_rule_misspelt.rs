use amethystate_macros::amethystate;

#[amethystate(prefix = "editor", on_unreadable = UseDefaults)]
pub struct Editor {
    #[amestate(default = 1280)]
    pub width: u32,
}

fn main() {}
