# tauri-yew

The todo app every example in this repository is, as a Tauri v2 app with a Yew frontend in WASM.
The store lives in the Rust backend; the frontend reaches it through `tauri-plugin-amethystate`.

```bash
cargo tauri dev
```

After changing `Todos` in `src-tauri/src/lib.rs`, regenerate the frontend's bindings:

```bash
cd src-tauri && cargo run --bin codegen
```
