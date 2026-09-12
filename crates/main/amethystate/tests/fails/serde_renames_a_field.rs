use amethystate_macros::amethystate;

#[amethystate(prefix = "ui")]
pub struct UiState {
    #[serde(rename = "listen_port")]
    #[amestate(default = 8080u16)]
    pub port: u16,
}

fn main() {}
