use amethystate_macros::amethystate;

#[amethystate(prefix = "ui")]
#[serde(deny_unknown_fields)]
pub struct UiState {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

fn main() {}
