use amethystate::store::builder::Backend;
use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use amethystate_dioxus::{
    AmeStateProvider, Handle, amethystate_framework_arena, use_amethystate, use_field, use_map,
    use_map_entry,
};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TodoList {
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    pub list: String,
    pub title: String,
    pub done: bool,
}

#[amethystate_framework_arena]
#[amethystate(prefix = "todos")]
pub struct Todos {
    #[amestate(default = 1u64)]
    pub next_id: u64,

    #[amestate(default = false)]
    pub hide_done: bool,

    #[amestate(default = {})]
    pub lists: ReactiveMap<String, TodoList>,

    #[amestate(default = {})]
    pub items: ReactiveMap<String, Todo>,
}

fn in_order<V>(mut entries: Vec<(String, V)>) -> Vec<(String, V)> {
    entries.sort_by_key(|(id, _)| id.parse::<u64>().unwrap_or(u64::MAX));
    entries
}

#[derive(Clone, PartialEq)]
enum Page {
    Overview,
    List(String),
    Settings,
}

fn use_minter(todos: Handle<Todos>) -> impl Fn() -> String + Copy {
    let (next, set_next) = use_field(todos.next_id);
    move || {
        let id = *next.peek();
        set_next.call(id + 1);
        id.to_string()
    }
}

#[component]
fn Overview(todos: Handle<Todos>, page: Signal<Page>) -> Element {
    let lists = use_map(todos.lists);
    let items = use_map(todos.items);
    let mint = use_minter(todos);
    let mut draft = use_signal(String::new);
    let mut page = page;

    let mut add = move || {
        let name = draft.peek().trim().to_string();
        if !name.is_empty() {
            lists.insert(mint(), TodoList { name });
        }
        draft.set(String::new());
    };

    let remove = move |list: String| {
        let doomed: Vec<String> = items
            .entries
            .peek()
            .iter()
            .filter(|(_, todo)| todo.list == list)
            .map(|(id, _)| id.clone())
            .collect();
        for id in doomed {
            items.remove(id);
        }
        lists.remove(list);
    };

    let entries = items.entries.read().clone();
    let tally = move |list: &str| {
        let mine: Vec<&Todo> = entries.values().filter(|todo| todo.list == list).collect();
        let done = mine.iter().filter(|todo| todo.done).count();
        format!("{done}/{}", mine.len())
    };

    rsx! {
        h2 { "lists" }
        for (id, list) in in_order(lists.entries.read().clone().into_iter().collect()) {
            div { key: "{id}",
                button {
                    onclick: {
                        let id = id.clone();
                        move |_| page.set(Page::List(id.clone()))
                    },
                    "{list.name}"
                }
                " {tally(&id)} "
                button {
                    onclick: {
                        let id = id.clone();
                        move |_| remove(id.clone())
                    },
                    "✕"
                }
            }
        }
        input {
            value: "{draft}",
            oninput: move |e| draft.set(e.value()),
            onkeydown: move |e| {
                if e.key() == Key::Enter {
                    add()
                }
            },
        }
        button { onclick: move |_| add(), "add list" }
    }
}

#[component]
fn Row(
    todos: Handle<Todos>,
    id: String,
    on_toggle: Callback<Todo>,
    on_remove: Callback<()>,
) -> Element {
    let todo = use_map_entry(todos.items, id);

    match todo.read().clone() {
        Some(held) => {
            let toggled = held.clone();
            rsx! {
                div {
                    label {
                        input {
                            r#type: "checkbox",
                            checked: held.done,
                            onchange: move |_| on_toggle.call(toggled.clone()),
                        }
                        " {held.title}"
                    }
                    " "
                    button { onclick: move |_| on_remove.call(()), "✕" }
                }
            }
        }
        None => rsx! {
            div { "(removed)" }
        },
    }
}

#[component]
fn ListPage(todos: Handle<Todos>, list: String) -> Element {
    let name = use_map_entry(todos.lists, list.clone());
    let items = use_map(todos.items);
    let (hide_done, _) = use_field(todos.hide_done);
    let mint = use_minter(todos);
    let mut draft = use_signal(String::new);
    let list = use_signal(|| list);

    let mut add = move || {
        let title = draft.peek().trim().to_string();
        if !title.is_empty() {
            let todo = Todo {
                list: list.peek().clone(),
                title,
                done: false,
            };
            items.insert(mint(), todo);
        }
        draft.set(String::new());
    };

    let clear_done = move |_| {
        let mine = list.peek().clone();
        let done: Vec<String> = items
            .entries
            .peek()
            .iter()
            .filter(|(_, todo)| todo.list == mine && todo.done)
            .map(|(id, _)| id.clone())
            .collect();
        for id in done {
            items.remove(id);
        }
    };

    let mine = list.read().clone();
    let hide = *hide_done.read();
    let entries = items.entries.read().clone();
    let left = entries
        .values()
        .filter(|todo| todo.list == mine && !todo.done)
        .count();
    let shown = in_order(
        entries
            .into_iter()
            .filter(|(_, todo)| todo.list == mine && !(hide && todo.done))
            .collect(),
    );

    let Some(found) = name.read().clone() else {
        return rsx! {
            p { "this list is gone" }
        };
    };

    rsx! {
        h2 { "{found.name}" }
        input {
            value: "{draft}",
            oninput: move |e| draft.set(e.value()),
            onkeydown: move |e| {
                if e.key() == Key::Enter {
                    add()
                }
            },
        }
        button { onclick: move |_| add(), "add" }
        for (id, _) in shown {
            Row {
                key: "{id}",
                todos,
                id: id.clone(),
                on_toggle: {
                    let id = id.clone();
                    move |todo: Todo| items.set(id.clone(), Todo { done: !todo.done, ..todo })
                },
                on_remove: {
                    let id = id.clone();
                    move |_: ()| items.remove(id.clone())
                },
            }
        }
        hr {}
        "{left} left "
        button { onclick: clear_done, "clear done" }
    }
}

#[component]
fn SettingsPage(todos: Handle<Todos>) -> Element {
    let (hide_done, set_hide_done) = use_field(todos.hide_done);

    rsx! {
        h2 { "settings" }
        label {
            input {
                r#type: "checkbox",
                checked: *hide_done.read(),
                onchange: move |e| set_hide_done.call(e.checked()),
            }
            " hide done"
        }
    }
}

#[component]
fn Shell() -> Element {
    let todos = use_amethystate::<Todos>();
    let mut page = use_signal(|| Page::Overview);

    rsx! {
        nav {
            button { onclick: move |_| page.set(Page::Overview), "lists" }
            " "
            button { onclick: move |_| page.set(Page::Settings), "settings" }
        }
        hr {}
        match page.read().clone() {
            Page::Overview => rsx! {
                Overview { todos, page }
            },
            Page::List(list) => rsx! {
                ListPage { key: "{list}", todos, list }
            },
            Page::Settings => rsx! {
                SettingsPage { todos }
            },
        }
    }
}

#[component]
fn App() -> Element {
    let store = use_hook(|| {
        StoreBuilder::new("todos")
            .backend(Backend::LocalStorage)
            .build()
            .expect("the page's storage would not open")
    });

    rsx! {
        AmeStateProvider { store, Shell {} }
    }
}

fn main() {
    dioxus::launch(App);
}
