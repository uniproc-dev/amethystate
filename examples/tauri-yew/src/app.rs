use amethystate::client::{Field, ReactiveMap};
use amethystate::tauri::TauriBackend;
use amethystate_yew::{
    preload_slices, use_amethystate, use_field, use_map,
    AmeStateProvider,
};

use shared::ProxyProfile;
use yew::prelude::*;
use crate::bindings::{AppSettings, Theme};

#[derive(Properties, PartialEq)]
struct EnvMapProps {
    env: ReactiveMap<String, String>,
}

#[function_component(EnvMapEditor)]
fn env_map_editor(props: &EnvMapProps) -> Html {
    let map = use_map(props.env.clone());

    let new_key = use_state(String::new);
    let new_val = use_state(String::new);

    let on_add = {
        let map = map.clone();
        let new_key = new_key.clone();
        let new_val = new_val.clone();

        Callback::from(move |_| {
            let key = (*new_key).clone();
            let val = (*new_val).clone();
            if key.is_empty() {
                return;
            }
            map.insert(key, val);
            new_key.set(String::new());
            new_val.set(String::new());
        })
    };

    let on_key_input = {
        let new_key = new_key.clone();
        Callback::from(move |e: InputEvent| {
            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
            new_key.set(el.value());
        })
    };

    let on_val_input = {
        let new_val = new_val.clone();
        Callback::from(move |e: InputEvent| {
            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
            new_val.set(el.value());
        })
    };

    html! {
        <div class="section">
            <h3>{"Environment Variables"}</h3>
            { for map.entries.iter().map(|(k, v)| {
                let key = k.clone();
                let on_remove = {
                    let map = map.clone();
                    let key = key.clone();
                    Callback::from(move |_| map.remove(key.clone()))
                };

                html! {
                    <div class="env-row" key={key}>
                        <code>{k}</code>
                        <span>{" = "}</span>
                        <code>{v}</code>
                        <button onclick={on_remove}>{"✕"}</button>
                    </div>
                }
            })}
            <div class="env-add">
                <input
                    placeholder="KEY"
                    value={(*new_key).clone()}
                    oninput={on_key_input}
                />
                <input
                    placeholder="value"
                    value={(*new_val).clone()}
                    oninput={on_val_input}
                />
                <button onclick={on_add}>{"Add"}</button>
            </div>
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct ThemeEditorProps {
    theme: Theme,
}

#[function_component(ThemeEditor)]
fn theme_editor(props: &ThemeEditorProps) -> Html {
    let (mode, set_mode) = use_field(props.theme.mode.clone());
    let (bg, set_bg) = use_field(props.theme.background.clone());
    let (fg, set_fg) = use_field(props.theme.foreground.clone());

    html! {
        <div class="section">
            <h3>{"Theme (nested)"}</h3>
            <div class="field">
                <label>{"Mode"}</label>
                <select
                    value={mode}
                    onchange={Callback::from(move |e: Event| {
                        if let Some(input) = e.target_dyn_into::<web_sys::HtmlInputElement>() {
                            set_mode.emit(input.value());
                        }
                    })}
                >
                    <option value="light">{"light"}</option>
                    <option value="dark">{"dark"}</option>
                </select>
            </div>
            <div class="field">
                <label>{"Background"}</label>
                <input
                    type="color"
                    value={bg}
                    oninput={Callback::from(move |e: InputEvent| {
                        let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                        set_bg.emit(el.value());
                    })}
                />
            </div>
            <div class="field">
                <label>{"Foreground"}</label>
                <input
                    type="color"
                    value={fg}
                    oninput={Callback::from(move |e: InputEvent| {
                        let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                        set_fg.emit(el.value());
                    })}
                />
            </div>
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct ProxyEditorProps {
    proxy: Field<ProxyProfile>,
}

#[function_component(ProxyEditor)]
fn proxy_editor(props: &ProxyEditorProps) -> Html {
    let (prof, set_prof) = use_field(props.proxy.clone());

    let is_enabled = prof.enabled;
    let status_color = if is_enabled { "green" } else { "red" };
    let status_text = if is_enabled { "active" } else { "inactive" };

    html! {
        <div class="section">
            <h3>{"Proxy (plain type)"}</h3>
            <div class="field">
                <label>{"Name"}</label>
                <input
                    value={prof.name.clone()}
                    oninput={{
                        let p = prof.clone();
                        let set_prof = set_prof.clone();
                        Callback::from(move |e: InputEvent| {
                            let mut p = p.clone();
                            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                            p.name = el.value();
                            set_prof.emit(p.clone());
                        })
                    }}
                />
            </div>
            <div class="field">
                <label>{"Address"}</label>
                <input
                    value={prof.address.clone()}
                    oninput={{
                        let p = prof.clone();
                        let set_prof = set_prof.clone();
                        Callback::from(move |e: InputEvent| {
                            let mut p = p.clone();
                            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                            p.address = el.value();
                            set_prof.emit(p.clone());
                        })
                    }}
                />
            </div>
            <div class="field">
                <label>{"Port"}</label>
                <input
                    type="number"
                    value={prof.port.to_string()}
                    oninput={{
                        let p = prof.clone();
                        let set_prof = set_prof.clone();
                        Callback::from(move |e: InputEvent| {
                            let mut p = p.clone();
                            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                            if let Ok(port) = el.value().parse::<u16>() {
                                p.port = port;
                                set_prof.emit(p.clone());
                            }
                        })
                    }}
                />
            </div>
            <div class="field">
                <label>
                    <input
                        type="checkbox"
                        checked={prof.enabled}
                        onchange={{
                            let p = prof.clone();
                            let set_prof = set_prof.clone();
                            Callback::from(move |e: Event| {
                                let mut p = p.clone();
                                let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                                p.enabled = el.checked();
                                set_prof.emit(p.clone());
                            })
                        }}
                    />
                    {" Enabled"}
                </label>
            </div>
            <p style={format!("color: {}", status_color)}>
                {format!("{}:{} — {}", prof.address, prof.port, status_text)}
            </p>
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct SettingsProps {
    state: AppSettings,
}

#[function_component(Settings)]
pub fn settings(props: &SettingsProps) -> Html {
    let (username, set_username) = use_field(props.state.username.clone());
    let (counter, set_counter) = use_field(props.state.counter.clone());

    let address = format!("{username}:{counter}");

    html! {
        <div>
            <h1>{"amethystate + Yew"}</h1>

            <div class="section">
                <h3>{"Basic fields"}</h3>
                <div class="field">
                    <label>{"Username"}</label>
                    <input
                        value={username}
                        oninput={Callback::from(move |e: InputEvent| {
                            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                            set_username.emit(el.value());
                        })}
                    />
                </div>
                <div class="field">
                    <label>{"Counter"}</label>
                    <input
                        type="number"
                        value={counter.to_string()}
                        oninput={Callback::from(move |e: InputEvent| {
                            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
                            if let Ok(n) = el.value().parse::<i32>() {
                                set_counter.emit(n);
                            }
                        })}
                    />
                </div>
                <p>{"Address → "} <strong>{address}</strong></p>
            </div>

            <ThemeEditor theme={props.state.theme.clone()} />
            <ProxyEditor proxy={props.state.proxy.clone()} />
            <EnvMapEditor env={props.state.env.clone()} />
        </div>
    }
}

#[function_component(MainLayout)]
fn main_layout() -> Html {
    let state = use_amethystate::<AppSettings>();

    html! {
        <Settings state={state} />
    }
}

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
