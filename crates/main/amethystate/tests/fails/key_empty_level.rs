use amethystate_macros::amethystate;

#[amethystate(prefix = "ui")]
pub struct UiState {
    #[amestate(default = 1280, key = "window..width")]
    pub width: u32,
}

fn main() {}
