use amethystate::{ReactiveScope, StoreBuilder, amethystate};
use slint::ComponentHandle;
use std::sync::Arc;

slint::include_modules!();

#[amethystate(prefix = "slint_settings")]
pub struct SettingsState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080)]
    pub port: u16,
}

fn main() -> Result<(), slint::PlatformError> {
    let store = StoreBuilder::new("./slint-settings.redb")
        .build()
        .expect("failed to open store");
    let state = SettingsState::new(&store).expect("failed to create settings");

    let ui = AppWindow::new()?;
    ui.set_host(state.host().get().into());
    ui.set_port_text(state.port().get().to_string().into());

    let address_of = |state: &SettingsState| {
        format!("{}:{}", state.host().get(), state.port().get())
    };

    ui.set_address(address_of(&state).into());

    let state_for_apply = state.clone();
    ui.on_apply(move |host, port_text| {
        let _ = state_for_apply.host().set(host.to_string());
        if let Ok(port) = port_text.parse::<u16>() {
            let _ = state_for_apply.port().set(port);
        }
    });

    let mut scope = ReactiveScope::new();
    let ui_weak = ui.as_weak();
    let state_for_address = state.clone();

    let show_address = Arc::new(move || {
        let address = format!(
            "{}:{}",
            state_for_address.host().get(),
            state_for_address.port().get()
        );
        let ui_weak = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_address(address.into());
            }
        });
    });

    let on_host = Arc::clone(&show_address);
    scope.watch(state.host().subscribe(move |_| on_host()));

    let on_port = Arc::clone(&show_address);
    scope.watch(state.port().subscribe(move |_| on_port()));

    ui.run()
}
