use amethystate_macros::amethystate;

#[amethystate(prefix = "ui")]
pub struct UiState {
    #[cfg(feature = "wide")]
    #[amestate(default = 1280u32)]
    pub width: u32,
}

fn main() {}
