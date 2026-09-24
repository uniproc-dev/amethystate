use amethystate::store::builder::Backend;
use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use amethystate_arena::amethystate_framework_arena;
use amethystate_leptos::{
    AmeStateProvider, Handle, use_amethystate, use_field, use_map, use_map_entry,
};
use leptos::prelude::*;
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

fn minter(todos: Handle<Todos>) -> impl Fn() -> String + Copy {
    let (next, set_next) = use_field(todos.next_id);
    move || {
        let id = next.get_untracked();
        set_next.set(id + 1);
        id.to_string()
    }
}

#[component]
fn Overview(todos: Handle<Todos>, page: RwSignal<Page>) -> impl IntoView {
    let lists = use_map(todos.lists);
    let items = use_map(todos.items);
    let mint = minter(todos);
    let (draft, set_draft) = signal(String::new());

    let add = move || {
        let name = draft.get_untracked().trim().to_string();
        if !name.is_empty() {
            lists.insert(mint(), TodoList { name });
        }
        set_draft.set(String::new());
    };

    let remove = move |list: String| {
        for (id, todo) in items.entries.get_untracked() {
            if todo.list == list {
                items.remove(id);
            }
        }
        lists.remove(list);
    };

    let tally = move |list: &str| {
        let entries = items.entries.get();
        let mine: Vec<&Todo> = entries.values().filter(|todo| todo.list == list).collect();
        let done = mine.iter().filter(|todo| todo.done).count();
        format!("{done}/{}", mine.len())
    };

    view! {
        <h2>"lists"</h2>
        <For
            each=move || in_order(lists.entries.get().into_iter().collect())
            key=|(id, list)| (id.clone(), list.name.clone())
            children=move |(id, list)| {
                let open = id.clone();
                let gone = id.clone();
                let counted = id.clone();
                view! {
                    <div>
                        <a href="#" on:click=move |e| {
                            e.prevent_default();
                            page.set(Page::List(open.clone()));
                        }>{list.name}</a>
                        " " {move || tally(&counted)} " "
                        <button on:click=move |_| remove(gone.clone())>"✕"</button>
                    </div>
                }
            }
        />
        <input
            prop:value=draft
            on:input=move |e| set_draft.set(event_target_value(&e))
            on:keydown=move |e| if e.key() == "Enter" { add() }
        />
        <button on:click=move |_| add()>"add list"</button>
    }
}

#[component]
fn Row(
    todos: Handle<Todos>,
    id: String,
    on_toggle: Callback<Todo>,
    on_remove: Callback<()>,
) -> impl IntoView {
    let todo = use_map_entry(todos.items, id);

    view! {
        {move || match todo.get() {
            Some(held) => {
                let toggled = held.clone();
                view! {
                    <div>
                        <label>
                            <input
                                type="checkbox"
                                prop:checked=held.done
                                on:change=move |_| on_toggle.run(toggled.clone())
                            />
                            " " {held.title}
                        </label>
                        " "
                        <button on:click=move |_| on_remove.run(())>"✕"</button>
                    </div>
                }
                .into_any()
            }
            None => view! { <div>"(removed)"</div> }.into_any(),
        }}
    }
}

#[component]
fn ListPage(todos: Handle<Todos>, list: String) -> impl IntoView {
    let name = use_map_entry(todos.lists, list.clone());
    let items = use_map(todos.items);
    let (hide_done, _) = use_field(todos.hide_done);
    let mint = minter(todos);
    let (draft, set_draft) = signal(String::new());
    let list = StoredValue::new(list);

    let add = move || {
        let title = draft.get_untracked().trim().to_string();
        if !title.is_empty() {
            let todo = Todo {
                list: list.get_value(),
                title,
                done: false,
            };
            items.insert(mint(), todo);
        }
        set_draft.set(String::new());
    };

    let shown = move || {
        let hide = hide_done.get();
        let mine = list.get_value();
        in_order(
            items
                .entries
                .get()
                .into_iter()
                .filter(|(_, todo)| todo.list == mine && !(hide && todo.done))
                .collect(),
        )
    };

    let left = move || {
        let mine = list.get_value();
        items
            .entries
            .get()
            .values()
            .filter(|todo| todo.list == mine && !todo.done)
            .count()
    };

    let clear_done = move |_| {
        let mine = list.get_value();
        for (id, todo) in items.entries.get_untracked() {
            if todo.list == mine && todo.done {
                items.remove(id);
            }
        }
    };

    view! {
        {move || match name.get() {
            None => view! { <p>"this list is gone"</p> }.into_any(),
            Some(found) => view! {
                <h2>{found.name}</h2>
                <input
                    prop:value=draft
                    on:input=move |e| set_draft.set(event_target_value(&e))
                    on:keydown=move |e| if e.key() == "Enter" { add() }
                />
                <button on:click=move |_| add()>"add"</button>
                <For
                    each=shown
                    key=|(id, _)| id.clone()
                    children=move |(id, _)| {
                        let toggled = id.clone();
                        let removed = id.clone();
                        let on_toggle = Callback::new(move |todo: Todo| {
                            items.set(toggled.clone(), Todo { done: !todo.done, ..todo });
                        });
                        let on_remove = Callback::new(move |_: ()| items.remove(removed.clone()));
                        view! { <Row todos=todos id=id on_toggle=on_toggle on_remove=on_remove /> }
                    }
                />
                <hr />
                {move || format!("{} left", left())} " "
                <button on:click=clear_done>"clear done"</button>
            }
            .into_any(),
        }}
    }
}

#[component]
fn SettingsPage(todos: Handle<Todos>) -> impl IntoView {
    let (hide_done, set_hide_done) = use_field(todos.hide_done);

    view! {
        <h2>"settings"</h2>
        <label>
            <input
                type="checkbox"
                prop:checked=hide_done
                on:change=move |e| set_hide_done.set(event_target_checked(&e))
            />
            " hide done"
        </label>
    }
}

#[component]
fn Shell() -> impl IntoView {
    let todos = use_amethystate::<Todos>();
    let page = RwSignal::new(Page::Overview);

    view! {
        <nav>
            <button on:click=move |_| page.set(Page::Overview)>"lists"</button>
            " "
            <button on:click=move |_| page.set(Page::Settings)>"settings"</button>
        </nav>
        <hr />
        {move || match page.get() {
            Page::Overview => view! { <Overview todos=todos page=page /> }.into_any(),
            Page::List(list) => view! { <ListPage todos=todos list=list /> }.into_any(),
            Page::Settings => view! { <SettingsPage todos=todos /> }.into_any(),
        }}
    }
}

fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Debug);

    let store = StoreBuilder::new("todos")
        .backend(Backend::LocalStorage)
        .build()
        .expect("the page's storage would not open");

    mount_to_body(move || {
        view! {
            <AmeStateProvider store=store>
                <Shell />
            </AmeStateProvider>
        }
    })
}
