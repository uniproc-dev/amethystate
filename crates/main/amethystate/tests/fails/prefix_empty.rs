use amethystate_macros::amethystate;

#[amethystate(prefix = "")]
pub struct UiState {
    #[amestate(default = 1280)]
    pub width: u32,
}

fn main() {}
