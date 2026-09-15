use amethystate_macros::amethystate;

#[amethystate(prefix = "ui")]
pub struct UiState {
    #[amestate(path = "one")]
    #[amestate(path = "another", default = 1280)]
    pub width: u32,
}

fn main() {}
