// GENERATED AUTOMATICALLY. DO NOT EDIT.
use amethystate_arena::amethystate_framework_arena;

#[amethystate_framework_arena]
#[::amethystate::amethystate(prefix = "todos", target = "tauri-wasm")]
pub struct Todos {
    pub next_id: u64,
    pub hide_done: bool,
    pub lists: ReactiveMap < String, TodoList >,
    pub items: ReactiveMap < String, Todo >,
}

