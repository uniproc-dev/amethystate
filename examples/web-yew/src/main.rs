use amethystate::store::builder::Backend;
use amethystate::{Field, ReactiveMap, StoreBuilder, amethystate};
use amethystate_yew::{AmeStateProvider, use_amethystate, use_field, use_map, use_map_entry};
use serde::{Deserialize, Serialize};
use web_sys::HtmlInputElement;
use yew::prelude::*;

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

fn mint(next_id: &Field<u64>) -> String {
    let id = next_id.get();
    let _ = next_id.set(id + 1);
    id.to_string()
}

#[derive(Clone, PartialEq)]
enum Page {
    Overview,
    List(String),
    Settings,
}

#[derive(Properties, PartialEq)]
struct DraftProps {
    label: AttrValue,
    on_submit: Callback<String>,
}

#[function_component]
fn Draft(props: &DraftProps) -> Html {
    let draft = use_state(String::new);

    let submit = {
        let draft = draft.clone();
        let on_submit = props.on_submit.clone();
        Callback::from(move |_: ()| {
            let text = draft.trim().to_string();
            if !text.is_empty() {
                on_submit.emit(text);
            }
            draft.set(String::new());
        })
    };

    let oninput = {
        let draft = draft.clone();
        Callback::from(move |e: InputEvent| {
            draft.set(e.target_unchecked_into::<HtmlInputElement>().value())
        })
    };
    let onkeydown = {
        let submit = submit.clone();
        Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Enter" {
                submit.emit(())
            }
        })
    };

    html! {
        <>
            <input value={(*draft).clone()} {oninput} {onkeydown} />
            <button onclick={move |_| submit.emit(())}>{ props.label.clone() }</button>
        </>
    }
}

#[derive(Properties, PartialEq)]
struct OverviewProps {
    next_id: Field<u64>,
    lists: ReactiveMap<String, TodoList>,
    items: ReactiveMap<String, Todo>,
    on_open: Callback<String>,
}

#[function_component]
fn Overview(props: &OverviewProps) -> Html {
    let lists = use_map(props.lists.clone());
    let items = use_map(props.items.clone());

    let add = {
        let lists = lists.clone();
        let next_id = props.next_id.clone();
        Callback::from(move |name: String| lists.insert(mint(&next_id), TodoList { name }))
    };

    let rows = in_order(lists.entries.clone()).into_iter().map(|(id, list)| {
        let mine: Vec<&Todo> = items.entries.iter().map(|(_, todo)| todo).filter(|todo| todo.list == id).collect();
        let done = mine.iter().filter(|todo| todo.done).count();
        let tally = format!(" {done}/{} ", mine.len());

        let open = {
            let on_open = props.on_open.clone();
            let id = id.clone();
            Callback::from(move |_| on_open.emit(id.clone()))
        };
        let remove = {
            let lists = lists.clone();
            let items = items.clone();
            let id = id.clone();
            Callback::from(move |_| {
                for (item, todo) in &items.entries {
                    if todo.list == id {
                        items.remove(item.clone());
                    }
                }
                lists.remove(id.clone());
            })
        };

        html! {
            <div key={id.clone()}>
                <button onclick={open}>{ list.name }</button>
                { tally }
                <button onclick={remove}>{ "✕" }</button>
            </div>
        }
    });

    html! {
        <>
            <h2>{ "lists" }</h2>
            { for rows }
            <Draft label="add list" on_submit={add} />
        </>
    }
}

#[derive(Properties, PartialEq)]
struct RowProps {
    items: ReactiveMap<String, Todo>,
    id: String,
    on_toggle: Callback<Todo>,
    on_remove: Callback<()>,
}

#[function_component]
fn Row(props: &RowProps) -> Html {
    let todo = use_map_entry(props.items.clone(), props.id.clone());

    match todo {
        Some(held) => {
            let toggle = {
                let on_toggle = props.on_toggle.clone();
                let held = held.clone();
                Callback::from(move |_| on_toggle.emit(held.clone()))
            };
            let remove = {
                let on_remove = props.on_remove.clone();
                Callback::from(move |_| on_remove.emit(()))
            };
            html! {
                <div>
                    <label>
                        <input type="checkbox" checked={held.done} onchange={toggle} />
                        { " " }{ held.title }
                    </label>
                    { " " }
                    <button onclick={remove}>{ "✕" }</button>
                </div>
            }
        }
        None => html! { <div>{ "(removed)" }</div> },
    }
}

#[derive(Properties, PartialEq)]
struct ListPageProps {
    next_id: Field<u64>,
    hide_done: Field<bool>,
    lists: ReactiveMap<String, TodoList>,
    items: ReactiveMap<String, Todo>,
    list: String,
}

#[function_component]
fn ListPage(props: &ListPageProps) -> Html {
    let name = use_map_entry(props.lists.clone(), props.list.clone());
    let items = use_map(props.items.clone());
    let (hide_done, _) = use_field(props.hide_done.clone());

    let Some(found) = name else {
        return html! { <p>{ "this list is gone" }</p> };
    };

    let add = {
        let items = items.clone();
        let next_id = props.next_id.clone();
        let list = props.list.clone();
        Callback::from(move |title: String| {
            let todo = Todo {
                list: list.clone(),
                title,
                done: false,
            };
            items.insert(mint(&next_id), todo)
        })
    };

    let mine = &props.list;
    let left = items
        .entries
        .iter()
        .filter(|(_, todo)| &todo.list == mine && !todo.done)
        .count();
    let shown = in_order(
        items
            .entries
            .iter()
            .filter(|(_, todo)| &todo.list == mine && !(hide_done && todo.done))
            .cloned()
            .collect(),
    );

    let rows = shown.into_iter().map(|(id, _)| {
        let on_toggle = {
            let items = items.clone();
            let id = id.clone();
            Callback::from(move |todo: Todo| {
                items.set(id.clone(), Todo { done: !todo.done, ..todo })
            })
        };
        let on_remove = {
            let items = items.clone();
            let id = id.clone();
            Callback::from(move |_: ()| items.remove(id.clone()))
        };
        html! {
            <Row key={id.clone()} items={props.items.clone()} id={id.clone()} {on_toggle} {on_remove} />
        }
    });

    let clear_done = {
        let items = items.clone();
        let list = props.list.clone();
        Callback::from(move |_| {
            for (id, todo) in &items.entries {
                if todo.list == list && todo.done {
                    items.remove(id.clone());
                }
            }
        })
    };

    html! {
        <>
            <h2>{ found.name }</h2>
            <Draft label="add" on_submit={add} />
            { for rows }
            <hr />
            { format!("{left} left ") }
            <button onclick={clear_done}>{ "clear done" }</button>
        </>
    }
}

#[derive(Properties, PartialEq)]
struct SettingsProps {
    hide_done: Field<bool>,
}

#[function_component]
fn SettingsPage(props: &SettingsProps) -> Html {
    let (hide_done, set_hide_done) = use_field(props.hide_done.clone());
    let onchange = Callback::from(move |e: Event| {
        set_hide_done.emit(e.target_unchecked_into::<HtmlInputElement>().checked())
    });

    html! {
        <>
            <h2>{ "settings" }</h2>
            <label>
                <input type="checkbox" checked={hide_done} {onchange} />
                { " hide done" }
            </label>
        </>
    }
}

#[function_component]
fn Shell() -> Html {
    let todos = use_amethystate::<Todos>();
    let page = use_state(|| Page::Overview);

    let go = |to: Page| {
        let page = page.clone();
        Callback::from(move |_| page.set(to.clone()))
    };
    let on_open = {
        let page = page.clone();
        Callback::from(move |list: String| page.set(Page::List(list)))
    };

    let body = match (*page).clone() {
        Page::Overview => html! {
            <Overview
                next_id={todos.next_id().clone()}
                lists={todos.lists().clone()}
                items={todos.items().clone()}
                {on_open}
            />
        },
        Page::List(list) => html! {
            <ListPage
                key={list.clone()}
                next_id={todos.next_id().clone()}
                hide_done={todos.hide_done().clone()}
                lists={todos.lists().clone()}
                items={todos.items().clone()}
                list={list.clone()}
            />
        },
        Page::Settings => html! { <SettingsPage hide_done={todos.hide_done().clone()} /> },
    };

    html! {
        <>
            <nav>
                <button onclick={go(Page::Overview)}>{ "lists" }</button>
                { " " }
                <button onclick={go(Page::Settings)}>{ "settings" }</button>
            </nav>
            <hr />
            { body }
        </>
    }
}

#[function_component]
fn App() -> Html {
    let store = use_memo((), |_| {
        StoreBuilder::new("todos")
            .backend(Backend::LocalStorage)
            .build()
            .expect("the page's storage would not open")
    });

    html! {
        <AmeStateProvider store={(*store).clone()}>
            <Shell />
        </AmeStateProvider>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
