use amethystate::StoreBuilder;
use amethystate::amethystate;

#[path = "../report.rs"]
mod report;

use report::anyhowed;

#[amethystate(prefix = "network", version = 1)]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080)]
    pub port: u16,
}

#[amethystate(prefix = "ui", version = 1)]
pub struct UiState {
    #[amestate(default = "dark".to_string())]
    pub theme: String,

    #[amestate(default = true)]
    pub sidebar_visible: bool,
}

fn main() -> anyhow::Result<()> {
    let store = StoreBuilder::new("./test_data").build().map_err(anyhowed)?;

    let network = NetworkState::new_with(&store).map_err(anyhowed)?;
    network
        .host()
        .set("10.0.0.1".to_string())
        .map_err(anyhowed)?;
    network.port().set(9090).map_err(anyhowed)?;

    let ui = UiState::new_with(&store).map_err(anyhowed)?;
    ui.theme().set("light".to_string()).map_err(anyhowed)?;

    println!("produced test_data.toml");
    Ok(())
}
