use dioxus::prelude::*;
use futures_util::StreamExt;

use amethystate::amethystate;
use amethystate_dioxus::{
    amethystate_dioxus, use_field, use_map, use_map_entry,
    use_amethystate, Handle, MapChange, MapHandle, ReactiveMap, ReadOnlyMapHandle,
    amethystateProvider, AmeType, StoreBuilder, WritableMapHandle
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, AmeType)]
pub struct ProxyProfile {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub enabled: bool,
}

impl Default for ProxyProfile {
    fn default() -> Self {
        Self {
            name: "Default Proxy".to_string(),
            address: "127.0.0.1".to_string(),
            port: 8080,
            enabled: false,
        }
    }
}

#[amethystate_dioxus]
#[amethystate(prefix = "settings")]
pub struct AppSettings {
    #[amestate(default = "Guest".to_string())]
    pub username: String,

    #[amestate(default = 0)]
    pub counter: i32,

    #[amestate(nested)]
    pub theme: Theme,

    #[amestate(default = Default::default())]
    pub proxy: ProxyProfile,

    #[amestate(default = {
        "HTTP_PROXY": "http://127.0.0.1:8080".to_string(),
        "NO_PROXY": "localhost".to_string()
    })]
    pub env: ReactiveMap<String, String>,
}

#[amethystate_dioxus]
#[amethystate]
pub struct Theme {
    #[amestate(default = "light".to_string())]
    pub mode: String,

    #[amestate(default = "#ffffff".to_string())]
    pub background: String,

    #[amestate(default = "#000000".to_string())]
    pub foreground: String,
}

#[component]
fn EnvMapEditor(env: WritableMapHandle<String, String>) -> Element {
    let map = use_map(env);
    let mut new_key = use_signal(String::new);
    let mut new_val = use_signal(String::new);

    let on_add = use_callback(move |_| {
        let key = new_key.peek().clone();
        let val = new_val.peek().clone();
        if key.is_empty() {
            return;
        }
        map.insert(key, val);
        new_key.set(String::new());
        new_val.set(String::new());
    });

    let on_remove = use_callback(move |key: String| {
        map.remove(key);
    });

    rsx! {
        div { class: "section",
            h3 { "Environment Variables" }
            for (k, v) in map.entries.read().clone() {
                div { class: "env-row",
                    code { "{k}" }
                    span { " = " }
                    code { "{v}" }
                    button { onclick: move |_| on_remove(k.clone()), "✕" }
                }
            }
            div { class: "env-add",
                input {
                    placeholder: "KEY",
                    value: "{new_key}",
                    oninput: move |e| new_key.set(e.value()),
                }
                input {
                    placeholder: "value",
                    value: "{new_val}",
                    oninput: move |e| new_val.set(e.value()),
                }
                button { onclick: on_add, "Add" }
            }
        }
    }
}

#[component]
fn ThemeEditor(settings: Handle<AppSettings>) -> Element {
    let (mode, set_mode) = use_field(settings.theme.mode);
    let (bg, set_bg) = use_field(settings.theme.background);
    let (fg, set_fg) = use_field(settings.theme.foreground);

    rsx! {
        div { class: "section",
            h3 { "Theme (nested)" }
            div { class: "field",
                label { "Mode" }
                select {
                    value: "{mode}",
                    onchange: move |e| set_mode(e.value()),
                    option { value: "light", "light" }
                    option { value: "dark", "dark" }
                }
            }
            div { class: "field",
                label { "Background" }
                input {
                    r#type: "color",
                    value: "{bg}",
                    oninput: move |e| set_bg(e.value()),
                }
            }
            div { class: "field",
                label { "Foreground" }
                input {
                    r#type: "color",
                    value: "{fg}",
                    oninput: move |e| set_fg(e.value()),
                }
            }
        }
    }
}

#[component]
fn ProxyEditor(settings: Handle<AppSettings>) -> Element {
    let (prof, set_prof) = use_field(settings.proxy);

    let status = move || {
        let p = prof.read();
        format!(
            "{}:{} — {}",
            p.address,
            p.port,
            if p.enabled { "active" } else { "inactive" }
        )
    };

    rsx! {
        div { class: "section",
            h3 { "Proxy (plain type)" }
            div { class: "field",
                label { "Name" }
                input {
                    value: "{prof.read().name}",
                    oninput: move |e| {
                        let mut p = prof.peek().clone();
                        p.name = e.value();
                        set_prof(p);
                    },
                }
            }
            div { class: "field",
                label { "Address" }
                input {
                    value: "{prof.read().address}",
                    oninput: move |e| {
                        let mut p = prof.peek().clone();
                        p.address = e.value();
                        set_prof(p);
                    },
                }
            }
            div { class: "field",
                label { "Port" }
                input {
                    r#type: "number",
                    value: "{prof.read().port}",
                    oninput: move |e| {
                        if let Ok(port) = e.value().parse::<u16>() {
                            let mut p = prof.peek().clone();
                            p.port = port;
                            set_prof(p);
                        }
                    },
                }
            }
            div { class: "field",
                label {
                    input {
                        r#type: "checkbox",
                        checked: prof.read().enabled,
                        onchange: move |e| {
                            let mut p = prof.peek().clone();
                            p.enabled = e.checked();
                            set_prof(p);
                        },
                    }
                    " Enabled"
                }
            }
            p {
                style: if prof.read().enabled { "color: green" } else { "color: red" },
                { status() }
            }
        }
    }
}

#[component]
fn Settings() -> Element {
    let state = use_amethystate::<AppSettings>();
    let (username, set_username) = use_field(state.username);
    let (counter, set_counter) = use_field(state.counter);

    rsx! {
        div {
            h1 { "amethystate + Dioxus" }

            div { class: "section",
                h3 { "Basic fields" }
                div { class: "field",
                    label { "Username" }
                    input {
                        value: "{username}",
                        oninput: move |e| set_username(e.value()),
                    }
                }
                div { class: "field",
                    label { "Counter" }
                    input {
                        r#type: "number",
                        value: "{counter}",
                        oninput: move |e| {
                            if let Ok(n) = e.value().parse::<i32>() {
                                set_counter(n);
                            }
                        },
                    }
                }
                p { "Address → " strong { "{username}:{counter}" } }
            }

            ThemeEditor { settings: state }
            ProxyEditor { settings: state }
            EnvMapEditor { env: state.env }
        }
    }
}

#[component]
fn App() -> Element {
    let store = use_hook(|| {
        StoreBuilder::new("./dioxus-settings")
            .build()
            .expect("failed to open store")
    });

    rsx! {
        amethystateProvider {
            store,
            Settings {}
        }
    }
}

fn main() {
    launch(App);
}
