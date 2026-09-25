use amethystate::StoreBuilder;
use amethystate::{amethystate, ReactiveMap};
use shared::{Todo, TodoList};

#[amethystate(prefix = "todos")]
pub struct Todos {
    #[amestate(default = 1u64)]
    pub next_id: u64,

    #[amestate(default = false)]
    pub hide_done: bool,

    #[amestate(default = {})]
    pub lists: ReactiveMap<String, TodoList>,

    #[amestate(default = {})]
    pub items: ReactiveMap<String, Todo>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store = StoreBuilder::located(|at| at.app("amethystate-examples", "tauri-leptos-todo"))
        .expect("there is no place for the store")
        .build()
        .expect("the store would not open");

    tauri::Builder::default()
        .plugin(tauri_plugin_amethystate::init(store))
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
