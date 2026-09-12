use amethystate_macros::amethystate;

#[amethystate(prefix = "ui")]
pub struct UiState {
    #[amestate(path = "window..width", default = 1280)]
    pub width: u32,
}

fn main() {}
