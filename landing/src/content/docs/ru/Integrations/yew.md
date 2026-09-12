---
title: Yew
---

`amethystate-yew` соединяет реактивное состояние с компонентами Yew. К каждому полю обращаются через хуки, возвращающие родные типы Yew: `T` для чтения и `Callback<T>` для записи. Компоненты перерисовываются только тогда, когда меняются те поля, которые они читают.

*Примечание: эта интеграция рассчитана на фронтенды на WASM (CSR), в первую очередь для приложений Tauri.*

## Установка

```toml
[dependencies]
amethystate-yew = { version = "0.20", features = ["tauri"] }
```

## Объявление состояния

Структуры фронтенда генерируются автоматически из ваших типов бэкенда через `amethystate-codegen`. Не пишите их руками.

```rust
// src/bindings/amethystate.rs
// GENERATED AUTOMATICALLY. DO NOT EDIT.

#[::amethystate::amethystate(prefix = "settings", target = "tauri-wasm")]
pub struct AppSettings {
    pub username: String,
    pub counter: i32,
    #[amestate(nested)]
    pub theme: Theme,
    pub proxy: ProxyProfile,
    pub env: ReactiveMap<String, String>,
}

#[::amethystate::amethystate(target = "tauri-wasm")]
pub struct Theme {
    pub mode: String,
    pub background: String,
    pub foreground: String,
}
```

Настройка кодогенерации - в главе [Интеграция с Tauri](./tauri).

## Провайдер и инициализация

Состояние загружается асинхронно по IPC. Оберните приложение в `AmeStateProvider` и объявите через `preload_slices!`, какие срезы загружать. Отрисовка приостановлена, пока не готовы все срезы.

```rust
use amethystate::tauri::TauriBackend;
use amethystate_yew::{preload_slices, AmeStateProvider};
use yew::prelude::*;

use crate::bindings::AppSettings;

#[function_component(App)]
pub fn app() -> Html {
    html! {
        <AmeStateProvider<TauriBackend>
            backend={TauriBackend::new()}
            init={preload_slices!(AppSettings)}
            fallback={html! { <p>{"Loading..."}</p> }}
        >
            <MainLayout />
        </AmeStateProvider<TauriBackend>>
    }
}
```

## Доступ к состоянию

Внутри компонента вызовите `use_amethystate::<S>()`, чтобы получить корневой хендл среза. Хендл - это структура с полем на каждое поле состояния; передавайте её вниз как проп.

```rust
#[function_component(MainLayout)]
fn main_layout() -> Html {
    let state = use_amethystate::<AppSettings>();

    html! {
        <Settings state={state} />
    }
}

#[derive(Properties, PartialEq)]
pub struct SettingsProps {
    state: AppSettings,
}
```

Вложенные структуры (помеченные `#[amestate(nested)]`) передают клонированием их хендла:

```rust
<ThemeEditor theme={props.state.theme.clone()} />
```

Обычные поля-структуры (например, `ProxyProfile`) и поля `ReactiveMap` обёрнуты в `Field<T>` и `ReactiveMap<K, V>` соответственно и тоже передаются клонированием:

```rust
<ProxyEditor proxy={props.state.proxy.clone()} />
<EnvMapEditor env={props.state.env.clone()} />
```

## Хуки

### use_field

Возвращает пару `(T, Callback<T>)` для поля, в которое можно писать. Сеттер применяет изменение оптимистично на фронтенде и асинхронно сохраняет его на бэкенде. Если бэкенд вернёт ошибку, значение сбрасывается к последнему подтверждённому состоянию.

```rust
let (username, set_username) = use_field(props.state.username.clone());

html! {
    <input
        value={username}
        oninput={Callback::from(move |e: InputEvent| {
            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
            set_username.emit(el.value());
        })}
    />
}
```

Для обычных полей-структур меняйте значение и заново отправляйте его целиком:

```rust
let (prof, set_prof) = use_field(props.proxy.clone());

// ...
let mut p = prof.clone();
p.name = el.value();
set_prof.emit(p);
```

### use_read_only_field

Возвращает `T` для любого поля, без сеттера рядом, - для значения, которое компонент показывает и никогда не пишет.

```rust
let host = use_read_only_field(state.host.clone());

html! {
    <p>{"Connected to: "}{host}</p>
}
```

### use_map

Возвращает `MapSignal<K, V>` для поля `ReactiveMap`, в которое можно писать. Сигнал держит снимок всех записей и обновляется на любое внешнее изменение. Он открывает `insert`, `set`, `remove` и `clear` прямыми методами.

```rust
#[derive(Properties, PartialEq)]
struct EnvMapProps {
    env: ReactiveMap<String, String>,
}

#[function_component(EnvMapEditor)]
fn env_map_editor(props: &EnvMapProps) -> Html {
    let map = use_map(props.env.clone());

    let on_add = {
        let map = map.clone();
        Callback::from(move |_| {
            map.insert("NEW_KEY".to_string(), "value".to_string());
        })
    };

    html! {
        <>
            { for map.entries.iter().map(|(k, v)| {
                let key = k.clone();
                let on_remove = {
                    let map = map.clone();
                    Callback::from(move |_| map.remove(key.clone()))
                };
                html! {
                    <div key={k.clone()}>
                        <code>{format!("{k} = {v}")}</code>
                        <button onclick={on_remove}>{"✕"}</button>
                    </div>
                }
            })}
            <button onclick={on_add}>{"Add Key"}</button>
        </>
    }
}
```

### use_map_entry

Подписывается на один ключ в `ReactiveMap`, возвращая `Option<V>`, который обновляется, когда этот ключ меняется.

```rust
let proxy_port = use_map_entry(state.env.clone(), "HTTP_PROXY".to_string());

html! {
    <p>{"Proxy Port: "}{proxy_port.unwrap_or_else(|| "Not set".into())}</p>
}
```

## Примеры

- [`tauri-yew`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-yew) — полноценное приложение Tauri v2 с фронтендом на Yew и WASM.
