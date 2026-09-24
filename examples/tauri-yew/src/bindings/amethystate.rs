// GENERATED AUTOMATICALLY. DO NOT EDIT.

#[::amethystate::amethystate(prefix = "todos", target = "tauri-wasm")]
pub struct Todos {
    pub next_id: u64,
    pub hide_done: bool,
    pub lists: ReactiveMap < String, TodoList >,
    pub items: ReactiveMap < String, Todo >,
}

