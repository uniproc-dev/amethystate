use amethystate::store::builder::Backend;
use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use futures::stream::{BoxStream, Stream, StreamExt, select_all};
use iced::widget::{button, checkbox, column, row, rule, text, text_input};
use iced::{Color, Element, Subscription, Task};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::hash::{Hash, Hasher};

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

type Done = Result<(), Box<dyn Error>>;

fn in_order<V>(mut entries: Vec<(String, V)>) -> Vec<(String, V)> {
    entries.sort_by_key(|(id, _)| id.parse::<u64>().unwrap_or(u64::MAX));
    entries
}

impl Todos {
    fn mint(&self) -> Result<String, Box<dyn Error>> {
        let id = self.next_id().get();
        self.next_id().set(id + 1)?;
        Ok(id.to_string())
    }

    pub fn add_list(&self, name: &str) -> Done {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        let id = self.mint()?;
        self.lists().insert(id, &TodoList { name: name.to_string() })?;
        Ok(())
    }

    pub fn remove_list(&self, list: &str) -> Done {
        for (id, todo) in self.items().entries() {
            if todo.list == list {
                self.items().remove(&id)?;
            }
        }
        self.lists().remove(list)?;
        Ok(())
    }

    pub fn add(&self, list: &str, title: &str) -> Done {
        let title = title.trim();
        if title.is_empty() {
            return Ok(());
        }
        let id = self.mint()?;
        let todo = Todo {
            list: list.to_string(),
            title: title.to_string(),
            done: false,
        };
        self.items().insert(id, &todo)?;
        Ok(())
    }

    pub fn toggle(&self, id: &str) -> Done {
        self.items().modify(id, |todo| todo.done = !todo.done)?;
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Done {
        self.items().remove(id)?;
        Ok(())
    }

    pub fn clear_done(&self, list: &str) -> Done {
        for (id, todo) in self.items().entries() {
            if todo.list == list && todo.done {
                self.items().remove(&id)?;
            }
        }
        Ok(())
    }

    pub fn all_lists(&self) -> Vec<(String, TodoList)> {
        in_order(self.lists().entries().collect())
    }

    pub fn shown_in(&self, list: &str) -> Vec<(String, Todo)> {
        let hide_done = self.hide_done().get();
        in_order(
            self.items()
                .entries()
                .filter(|(_, todo)| todo.list == list && !(hide_done && todo.done))
                .collect(),
        )
    }

    pub fn tally(&self, list: &str) -> (usize, usize) {
        let mine: Vec<Todo> = self
            .items()
            .entries()
            .map(|(_, todo)| todo)
            .filter(|todo| todo.list == list)
            .collect();
        (mine.iter().filter(|todo| todo.done).count(), mine.len())
    }
}

#[derive(Clone, Debug, PartialEq, Hash)]
enum Page {
    Overview,
    List(String),
    Settings,
}

#[derive(Clone, Debug)]
enum Message {
    Go(Page),
    Draft(String),
    AddList,
    RemoveList(String),
    Add(String),
    Toggle(String),
    Remove(String),
    ClearDone(String),
    HideDone(bool),
    Changed,
}

struct Watched {
    page: Page,
    todos: Todos,
}

impl Hash for Watched {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.page.hash(state);
    }
}

fn changes(watched: &Watched) -> impl Stream<Item = Message> + use<> {
    let todos = &watched.todos;
    let streams: Vec<BoxStream<'static, ()>> = match &watched.page {
        Page::Overview => vec![
            todos.lists().subscription_with().stream().map(|_| ()).boxed(),
            todos.items().subscription_with().stream().map(|_| ()).boxed(),
        ],
        Page::List(list) => vec![
            todos
                .lists()
                .subscription_with()
                .key(list.clone())
                .stream()
                .map(|_| ())
                .boxed(),
            todos.items().subscription_with().stream().map(|_| ()).boxed(),
            todos.hide_done().subscription_with().stream().map(|_| ()).boxed(),
        ],
        Page::Settings => vec![
            todos.hide_done().subscription_with().stream().map(|_| ()).boxed(),
        ],
    };
    select_all(streams).map(|()| Message::Changed)
}

struct App {
    todos: Todos,
    page: Page,
    draft: String,
    heard: u64,
    failed: Option<String>,
}

impl App {
    fn report(&mut self, done: Done) {
        if let Err(why) = done {
            self.failed = Some(why.to_string());
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Go(page) => {
                self.page = page;
                self.draft.clear();
                self.heard = 0;
            }
            Message::Draft(draft) => self.draft = draft,
            Message::AddList => {
                let added = self.todos.add_list(&self.draft);
                self.report(added);
                self.draft.clear();
            }
            Message::RemoveList(list) => {
                let removed = self.todos.remove_list(&list);
                self.report(removed);
            }
            Message::Add(list) => {
                let added = self.todos.add(&list, &self.draft);
                self.report(added);
                self.draft.clear();
            }
            Message::Toggle(id) => {
                let toggled = self.todos.toggle(&id);
                self.report(toggled);
            }
            Message::Remove(id) => {
                let removed = self.todos.remove(&id);
                self.report(removed);
            }
            Message::ClearDone(list) => {
                let cleared = self.todos.clear_done(&list);
                self.report(cleared);
            }
            Message::HideDone(hide) => {
                let hidden = self.todos.hide_done().set(hide).map_err(Into::into);
                self.report(hidden);
            }
            Message::Changed => self.heard += 1,
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run_with(
            Watched {
                page: self.page.clone(),
                todos: self.todos.clone(),
            },
            changes,
        )
    }

    fn overview(&self) -> Element<'_, Message> {
        let mut page = column![text("lists").size(24)].spacing(8);

        for (id, list) in self.todos.all_lists() {
            let (done, total) = self.todos.tally(&id);
            page = page.push(
                row![
                    button(text(list.name))
                        .style(button::text)
                        .on_press(Message::Go(Page::List(id.clone()))),
                    text(format!("{done}/{total}")),
                    button("✕").on_press(Message::RemoveList(id)),
                ]
                .spacing(8),
            );
        }

        page.push(
            row![
                text_input("", &self.draft)
                    .on_input(Message::Draft)
                    .on_submit(Message::AddList),
                button("add list").on_press(Message::AddList),
            ]
            .spacing(8),
        )
        .into()
    }

    fn list(&self, id: &str) -> Element<'_, Message> {
        let Some(list) = self.todos.lists().get(id) else {
            return text("this list is gone").into();
        };

        let mut page = column![
            text(list.name).size(24),
            row![
                text_input("", &self.draft)
                    .on_input(Message::Draft)
                    .on_submit(Message::Add(id.to_string())),
                button("add").on_press(Message::Add(id.to_string())),
            ]
            .spacing(8),
        ]
        .spacing(8);

        for (item, todo) in self.todos.shown_in(id) {
            let toggled = item.clone();
            page = page.push(
                row![
                    checkbox(todo.done)
                        .label(todo.title)
                        .on_toggle(move |_| Message::Toggle(toggled.clone())),
                    button("✕").on_press(Message::Remove(item)),
                ]
                .spacing(8),
            );
        }

        let (done, total) = self.todos.tally(id);
        page.push(rule::horizontal(1))
            .push(
                row![
                    text(format!("{} left", total - done)),
                    button("clear done").on_press(Message::ClearDone(id.to_string())),
                ]
                .spacing(8),
            )
            .into()
    }

    fn settings(&self) -> Element<'_, Message> {
        column![
            text("settings").size(24),
            checkbox(self.todos.hide_done().get())
                .label("hide done")
                .on_toggle(Message::HideDone),
        ]
        .spacing(8)
        .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let body = match &self.page {
            Page::Overview => self.overview(),
            Page::List(id) => self.list(id),
            Page::Settings => self.settings(),
        };

        let mut page = column![
            row![
                button("lists").on_press(Message::Go(Page::Overview)),
                button("settings").on_press(Message::Go(Page::Settings)),
            ]
            .spacing(8),
            rule::horizontal(1),
            body,
            rule::horizontal(1),
            text(format!("changes this page has heard: {}", self.heard)).size(12),
        ]
        .padding(16)
        .spacing(12);

        if let Some(why) = &self.failed {
            page = page.push(text(why.clone()).color(Color::from_rgb(0.8, 0.1, 0.1)));
        }

        page.into()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let store = StoreBuilder::located(|at| at.app("amethystate-examples", "iced-todo"))?
        .backend(Backend::Json)
        .build()?;
    let todos = Todos::new_with(&store)?;

    iced::application(
        move || App {
            todos: todos.clone(),
            page: Page::Overview,
            draft: String::new(),
            heard: 0,
            failed: None,
        },
        App::update,
        App::view,
    )
    .subscription(App::subscription)
    .title("todos - amethystate + iced")
    .run()?;

    Ok(())
}
