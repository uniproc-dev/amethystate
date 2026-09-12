---
title: Tauri
---

`tauri-plugin-amethystate` - плагин для Tauri v2, который соединяет ваши срезы состояния с фронтендом Tauri по IPC. Он даёт команды для чтения, записи и подписки на состояние и несёт с собой генератор кода, который производит типизированные биндинги и для TypeScript, и для фронтендов на Rust и WASM.

## Общая картина

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (TypeScript or Rust/WASM)                         │
│  in-memory snapshot populated by load() and kept in         │
│  Field objects / structs                                    │
└────────────────────────┬────────────────────────────────────┘
                         │ IPC (Tauri commands)
┌────────────────────────▼────────────────────────────────────┐
│  amethystate (Rust)                                         │
│  in-memory write buffer, reactive subscriptions             │
└────────────────────────┬────────────────────────────────────┘
                         │ debounced flush / explicit save
┌────────────────────────▼────────────────────────────────────┐
│  Disk                                                       │
└─────────────────────────────────────────────────────────────┘
```

## Оптимистичные обновления

API фронтенда синхронный по замыслу: чтения и записи возвращаются сразу, не дожидаясь подтверждения по IPC. Значит, фронтенд применяет обновления оптимистично: сначала обновляется локальное значение, а вызов IPC к бэкенду идёт следом, асинхронно.

Если бэкенд вернёт ошибку, значение на фронтенде сбрасывается к последнему подтверждённому состоянию. Этот размен держит интерфейс отзывчивым, но означает, что упавшая запись заметно откатится.

## Установка

Добавьте плагин в Rust-крейт вашего приложения Tauri:

```toml
# src-tauri/Cargo.toml
[dependencies]
tauri-plugin-amethystate = "0.20"
```

`amethystate` реэкспортирован как `tauri_plugin_amethystate::amethystate`, поэтому отдельная зависимость не нужна.

Зарегистрируйте плагин и своё хранилище в `main.rs`:

```rust
use tauri_plugin_amethystate::amethystate::StoreBuilder;

fn main() {
    let store = StoreBuilder::new("./app").build().unwrap();

    tauri::Builder::default()
        .manage(store)
        .plugin(tauri_plugin_amethystate::init())
        .run(tauri::generate_context!())
        .unwrap();
}
```

## Разрешения

Добавьте набор разрешений по умолчанию в `src-tauri/capabilities/default.json`:

```json
{
  "permissions": [
    "amethystate:default"
  ]
}
```

`amethystate:default` включает следующие разрешения:

| Идентификатор | Описание |
|------------|-------------|
| `amethystate:allow-amethystate-get` | Прочитать один ключ |
| `amethystate:allow-amethystate-set` | Записать один ключ |
| `amethystate:allow-amethystate-delete` | Удалить один ключ |
| `amethystate:allow-amethystate-subscribe` | Подписаться на изменения ключа |
| `amethystate:allow-amethystate-unsubscribe` | Отписаться от ключа |
| `amethystate:allow-amethystate-get-prefix` | Прочитать пачкой все ключи под префиксом |
| `amethystate:allow-amethystate-flush` | Сбросить отложенные записи на диск |

У каждого разрешения есть парный вариант `deny-*`, и он имеет приоритет над `allow-*`.

## Кодогенерация

`amethystate-codegen` генерирует типизированные биндинги фронтенда из ваших структур `#[amethystate]`. Бинарник должен лежать в том же крейте, где объявлены ваши типы.

**1. Добавьте бинарную цель и зависимость:**

```toml
[[bin]]
name = "codegen"
path = "src/bin/codegen.rs"

[dependencies]
amethystate-codegen = { version = "0.20" }
```

Для фронтендов на Rust и WASM добавьте подходящий feature-флаг:

| Feature | Фреймворк |
|---------|-----------|
| `leptos` | Leptos |
| `dioxus` | Dioxus |
| `yew` | Yew |
| *(нет)* | TypeScript или чистый WASM |

**2. Создайте `src/bin/codegen.rs`:**

Для фронтенда на TypeScript:

```rust
#[allow(unused_imports)]
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    ts_out = "../src/bindings/amethystate.ts"
);
```

Для фронтенда на Rust и WASM:

```rust
#[allow(unused_imports)]
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    rs_out = "../src/bindings/amethystate.rs",
    framework = leptos
);
```

**3. Запустите:**

```sh
cargo run --bin codegen
```

## Примеры

- [`tauri-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-settings) — фронтенд на TypeScript
- [`tauri-leptos`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-leptos) — фронтенд на Leptos и WASM
- [`tauri-yew`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-yew) — фронтенд на Yew и WASM
