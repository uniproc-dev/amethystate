use tauri_typescript_lib::{Todo, TodoList};
use ts_rs::{Config, TS};

fn main() {
    let beside_the_bindings = Config::new().with_out_dir("../src/bindings");
    TodoList::export_all(&beside_the_bindings).expect("TodoList would not export");
    Todo::export_all(&beside_the_bindings).expect("Todo would not export");

    amethystate_codegen::amethystate_codegen!(ts_out = "../src/bindings/amethystate.ts");
}
