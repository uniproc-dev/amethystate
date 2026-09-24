# tauri-typescript

The todo app every example in this repository is, as a Tauri v2 app with a plain TypeScript
frontend over the `amethystate` npm package, taken from `../../js`.

```bash
npm --prefix ../../js install && npm --prefix ../../js run build
npm install
npx tauri dev
```

After changing `Todos`, `Todo` or `TodoList` in `src-tauri/src/lib.rs`, regenerate the bindings:
`ts-rs` writes the value types and `amethystate-codegen` the classes over them.

```bash
cd src-tauri && cargo run --bin codegen
```

The backend builds `Todos` when it starts, so the defaults declared in Rust are in the store
before the frontend loads it.
