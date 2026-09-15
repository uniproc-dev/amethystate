use eframe::egui;
use amethystate::store::builder::StoreBuilder;
use amethystate::amethystate;

#[amethystate(prefix = "egui_settings")]
pub struct SettingsState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080)]
    pub port: u16,

    #[amestate(default = false)]
    pub dark_mode: bool,
}

struct SettingsApp {
    state: SettingsState,
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.state.dark_mode().get() {
            ui.ctx().set_visuals(egui::Visuals::dark());
        } else {
            ui.ctx().set_visuals(egui::Visuals::light());
        }

        ui.heading("amethystate + egui");

        let mut host = self.state.host().get();
        if ui.text_edit_singleline(&mut host).changed() {
            let _ = self.state.host().set(host);
        }

        let mut port = self.state.port().get();
        if ui
            .add(egui::Slider::new(&mut port, 1024..=9999).text("port"))
            .changed()
        {
            let _ = self.state.port().set(port);
        }

        let mut dark_mode = self.state.dark_mode().get();
        if ui.checkbox(&mut dark_mode, "dark mode").changed() {
            let _ = self.state.dark_mode().set(dark_mode);
        }

        ui.separator();
        ui.label(format!("address: {host}:{port}"));
    }
}

fn main() -> anyhow::Result<()> {
    let store = StoreBuilder::new("./egui-settings.redb").build()?;
    let state = SettingsState::new(&store)?;

    eframe::run_native(
        "amethystate egui settings",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Ok(Box::new(SettingsApp { state }))),
    )?;

    Ok(())
}
