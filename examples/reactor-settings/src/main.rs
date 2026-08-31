use amethystate::{IntoGlobalStore, ReactiveMap, amethystate};
use amethystate_reactor::AmeCx;
use windows_reactor::*;

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = 8080)]
    pub port: u16,

    #[amestate(default = "dark".to_string())]
    pub theme: String,

    pub widths: ReactiveMap<String, u64>,
}

fn app(cx: &mut RenderCx) -> Element {
    let state: NetworkState = cx.use_ame_state();

    let port = cx.use_ame(&state.port());
    let theme = cx.use_ame(&state.theme());
    let cpu = cx.use_ame_entry(&state.widths(), "cpu".to_string());

    let bump = state.port();
    let reset = state.port();

    vstack((
        TitleBar::new("settings").subtitle(theme),
        text_block(format!("port = {port}")).font_size(28.0).bold(),
        text_block(format!("cpu column = {cpu:?}")),
        hstack((
            button("+1").on_click(move || {
                let _ = bump.set(port + 1);
            }),
            button("Reset").on_click(move || {
                let _ = reset.set(8080);
            }),
        ))
        .spacing(8.0),
    ))
    .spacing(16.0)
    .into()
}

fn main() -> Result<()> {
    bootstrap()?;
    let (_report, _ame) = "./settings.redb".init_global_with_migration();

    App::new()
        .title("settings")
        .inner_size(420.0, 320.0)
        .render(app)
}
