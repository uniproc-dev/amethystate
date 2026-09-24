use amethystate::{amethystate, ReactiveMap, StoreBuilder};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct TodoList {
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct Todo {
    pub list: String,
    pub title: String,
    pub done: bool,
}

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
    let store = StoreBuilder::located(|at| at.app("amethystate-examples", "tauri-typescript-todo"))
        .expect("there is no place for the store")
        .build()
        .expect("the store would not open");
    let _defaults_written = Todos::new_with(&store).expect("the todos would not load");

    tauri::Builder::default()
        .plugin(tauri_plugin_amethystate::init(store))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
