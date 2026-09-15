use crate::bindings::{AppSettings, Theme};
use leptos::prelude::*;
use amethystate::tauri::TauriBackend;
use amethystate_arena::{WritableHandle, WritableMapHandle};
use amethystate_leptos::{preload_slices, use_field, use_map, use_amethystate, Handle, amethystateProvider};
use shared::ProxyProfile;

#[component]
fn EnvMapEditor(env: WritableMapHandle<String, String>) -> impl IntoView {
    let map = use_map(env);
    let (new_key, set_key) = signal(String::new());
    let (new_val, set_val) = signal(String::new());

    let on_add = move |_| {
        let key = new_key.get();
        let val = new_val.get();
        if key.is_empty() {
            return;
        }
        map.insert(key, val);
        set_key.set(String::new());
        set_val.set(String::new());
    };

    let on_remove = Callback::new(move |key: String| {
        map.remove(key);
    });

    view! {
        <div class="section">
            <h3>"Environment Variables"</h3>
            <For
                each=move || map.entries.get()
                key=|(k, _)| k.clone()
                children=move |(k, v)| {
                    let key = k.clone();
                    view! {
                        <div class="env-row">
                            <code>{k}</code>
                            <span>" = "</span>
                            <code>{v}</code>
                            <button on:click=move |_| on_remove.run(key.clone())>"✕"</button>
                        </div>
                    }
                }
            />
            <div class="env-add">
                <input
                    placeholder="KEY"
                    prop:value=new_key
                    on:input=move |e| set_key.set(event_target_value(&e))
                />
                <input
                    placeholder="value"
                    prop:value=new_val
                    on:input=move |e| set_val.set(event_target_value(&e))
                />
                <button on:click=on_add>"Add"</button>
            </div>
        </div>
    }
}

#[component]
fn ThemeEditor(theme: Handle<Theme>) -> impl IntoView {
    let (mode, set_mode) = use_field(theme.mode);
    let (bg, set_bg) = use_field(theme.background);
    let (fg, set_fg) = use_field(theme.foreground);

    view! {
        <div class="section">
            <h3>"Theme (nested)"</h3>
            <div class="field">
                <label>"Mode"</label>
                <select
                    prop:value=move || {
                        let v = mode.get();
                        log::debug!("[view] mode read: {v}");
                        v
                    }
                    on:change=move |e| set_mode.set(event_target_value(&e))
                >
                    <option value="light">"light"</option>
                    <option value="dark">"dark"</option>
                </select>
            </div>
            <div class="field">
                <label>"Background"</label>
                <input
                    type="color"
                    prop:value=bg
                    on:input=move |e| set_bg.set(event_target_value(&e))
                />
            </div>
            <div class="field">
                <label>"Foreground"</label>
                <input
                    type="color"
                    prop:value=fg
                    on:input=move |e| set_fg.set(event_target_value(&e))
                />
            </div>
        </div>
    }
}

#[component]
fn ProxyEditor(proxy: WritableHandle<ProxyProfile>) -> impl IntoView {

    let (prof, set_prof) = use_field(proxy);

    view! {
        <div class="section">
            <h3>"Proxy (plain type)"</h3>
            <div class="field">
                <label>"Name"</label>
                <input
                    prop:value=move || prof.get().name
                    on:input=move |e| {
                        let mut p = prof.get_untracked();
                        p.name = event_target_value(&e);
                        set_prof.set(p);
                    }
                />
            </div>
            <div class="field">
                <label>"Address"</label>
                <input
                    prop:value=move || prof.get().address
                    on:input=move |e| {
                        let mut p = prof.get_untracked();
                        p.address = event_target_value(&e);
                        set_prof.set(p);
                    }
                />
            </div>
            <div class="field">
                <label>"Port"</label>
                <input
                    type="number"
                    prop:value=move || prof.get().port.to_string()
                    on:input=move |e| {
                        if let Ok(port) = event_target_value(&e).parse::<u16>() {
                            let mut p = prof.get_untracked();
                            p.port = port;
                            set_prof.set(p);
                        }
                    }
                />
            </div>
            <div class="field">
                <label>
                    <input
                        type="checkbox"
                        prop:checked=move || prof.get().enabled
                        on:change=move |e| {
                            let mut p = prof.get_untracked();
                            p.enabled = event_target_checked(&e);
                            set_prof.set(p);
                        }
                    />
                    " Enabled"
                </label>
            </div>
            <p style=move || format!(
                "color: {}",
                if prof.get().enabled { "green" } else { "red" }
            )>
                {move || format!("{}:{} — {}",
                    prof.get().address,
                    prof.get().port,
                    if prof.get().enabled { "active" } else { "inactive" }
                )}
            </p>
        </div>
    }
}

#[component]
pub fn Settings(state: Handle<AppSettings>) -> impl IntoView {
    let (username, set_username) = use_field(state.username);
    let (counter, set_counter) = use_field(state.counter);

    let address = move || format!("{}:{}", username.get(), counter.get());

    view! {
        <div>
            <h1>"amethystate + Leptos"</h1>

            <div class="section">
                <h3>"Basic fields"</h3>
                <div class="field">
                    <label>"Username"</label>
                    <input
                        prop:value=username
                        on:input=move |e| set_username.set(event_target_value(&e))
                    />
                </div>
                <div class="field">
                    <label>"Counter"</label>
                    <input
                        type="number"
                        prop:value=move || counter.get().to_string()
                        on:input=move |e| {
                            if let Ok(n) = event_target_value(&e).parse::<i32>() {
                                set_counter.set(n);
                            }
                        }
                    />
                </div>
                <p>"Address → " <strong>{address}</strong></p>
            </div>

            <ThemeEditor theme=state.theme />
            <ProxyEditor proxy=state.proxy />
            <EnvMapEditor env=state.env />
        </div>
    }
}

#[component]
pub fn App() -> impl IntoView {
    let backend = TauriBackend::new();

    view! {
        <amethystateProvider
            backend=backend
            init=preload_slices!(AppSettings)
            fallback=|| view! { <p>"Loading..."</p> }
        >
            <MainLayout />
        </amethystateProvider>
    }
}

#[component]
fn MainLayout() -> impl IntoView {
    let state = use_amethystate::<AppSettings>();

    view! {
        <Settings state=state />
    }
}