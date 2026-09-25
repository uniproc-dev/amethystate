use amethystate::amethystate;
use amethystate::client::AmeStateSliceAsync;
use amethystate::tauri::TauriBackend;

#[amethystate(target = "tauri-wasm")]
pub struct Layout {
    #[amestate(default = 1280)]
    pub width: u32,
}

#[amethystate(prefix = "editor", target = "tauri-wasm")]
pub struct Editor {
    #[amestate(default = "light".to_string())]
    pub theme: String,

    #[amestate(nested)]
    pub layout: Layout,

    #[amestate(default = {})]
    pub tabs: ReactiveMap<String, u32>,
}

async fn load(backend: &TauriBackend) -> Editor {
    Editor::load_async(backend).await.unwrap()
}

fn main() {
    let _ = load;
}
