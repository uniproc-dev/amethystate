use amethystate::store::builder::Backend;
use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use amethystate_gpui::{AmeStateExt, AmeEntity};
use gpui::{
    AnyView, App, AppContext, Context, ElementId, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, TitlebarOptions, WeakEntity, Window, WindowOptions, div,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{ActiveTheme, Root, h_flex, v_flex};
use serde::{Deserialize, Serialize};
use std::error::Error;

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

#[derive(Clone, PartialEq)]
enum Page {
    Overview,
    List(String),
    Settings,
}

fn held(todos: &AmeEntity<Todos>, cx: &App) -> Todos {
    (**todos.read(cx)).clone()
}

fn write(
    todos: &AmeEntity<Todos>,
    failed: &mut Option<SharedString>,
    cx: &mut App,
    change: impl FnOnce(&Todos) -> Done,
) {
    if let Err(why) = change(&held(todos, cx)) {
        *failed = Some(why.to_string().into());
    }
    todos.update(cx, |_, cx| cx.notify());
}

fn named(kind: &str, id: &str) -> ElementId {
    ElementId::Name(format!("{kind}-{id}").into())
}

fn footer(heard: u64, failed: &Option<SharedString>) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().child(SharedString::from(format!(
            "changes this page has heard: {heard}"
        ))))
        .children(failed.clone().map(|why| div().text_color(gpui::red()).child(why)))
}

struct Overview {
    todos: AmeEntity<Todos>,
    shell: WeakEntity<Shell>,
    draft: Entity<InputState>,
    heard: u64,
    failed: Option<SharedString>,
    _watching: Vec<Subscription>,
}

impl Overview {
    fn new(
        todos: AmeEntity<Todos>,
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = cx.new(|cx| InputState::new(window, cx).placeholder("new list"));
        let _watching = vec![
            cx.observe(&todos, |this, _, cx| {
                this.heard += 1;
                cx.notify();
            }),
            cx.subscribe_in(&draft, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.add(window, cx);
                }
            }),
        ];

        Self {
            todos,
            shell,
            draft,
            heard: 0,
            failed: None,
            _watching,
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.draft.read(cx).value();
        write(&self.todos, &mut self.failed, cx, |todos| todos.add_list(&name));
        self.draft.update(cx, |draft, cx| draft.set_value("", window, cx));
    }
}

impl Render for Overview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let todos = held(&self.todos, cx);

        let rows = todos.all_lists().into_iter().map(|(id, list)| {
            let (done, total) = todos.tally(&id);
            let shell = self.shell.clone();
            let opened = id.clone();
            let removed = id.clone();
            h_flex()
                .gap_2()
                .child(
                    Button::new(named("open", &id))
                        .ghost()
                        .label(list.name)
                        .on_click(move |_, window, cx| {
                            let page = Page::List(opened.clone());
                            let _ = shell.update(cx, |shell, cx| shell.go(page, window, cx));
                        }),
                )
                .child(SharedString::from(format!("{done}/{total}")))
                .child(Button::new(named("remove", &id)).label("✕").on_click(cx.listener(
                    move |this, _, _, cx| {
                        write(&this.todos, &mut this.failed, cx, |todos| {
                            todos.remove_list(&removed)
                        });
                    },
                )))
        });

        v_flex()
            .gap_2()
            .child(div().text_xl().child("lists"))
            .children(rows)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w_64().child(Input::new(&self.draft)))
                    .child(
                        Button::new("add-list")
                            .label("add list")
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
            .child(footer(self.heard, &self.failed))
    }
}

struct ListPage {
    todos: AmeEntity<Todos>,
    list: String,
    draft: Entity<InputState>,
    heard: u64,
    failed: Option<SharedString>,
    _watching: Vec<Subscription>,
}

impl ListPage {
    fn new(
        todos: AmeEntity<Todos>,
        list: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = cx.new(|cx| InputState::new(window, cx).placeholder("new todo"));
        let _watching = vec![
            cx.observe(&todos, |this, _, cx| {
                this.heard += 1;
                cx.notify();
            }),
            cx.subscribe_in(&draft, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.add(window, cx);
                }
            }),
        ];

        Self {
            todos,
            list,
            draft,
            heard: 0,
            failed: None,
            _watching,
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.draft.read(cx).value();
        let list = self.list.clone();
        write(&self.todos, &mut self.failed, cx, |todos| todos.add(&list, &title));
        self.draft.update(cx, |draft, cx| draft.set_value("", window, cx));
    }
}

impl Render for ListPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let todos = held(&self.todos, cx);

        let Some(found) = todos.lists().get(&self.list) else {
            return v_flex()
                .gap_2()
                .child("this list is gone")
                .child(footer(self.heard, &self.failed));
        };

        let rows = todos.shown_in(&self.list).into_iter().map(|(id, todo)| {
            let toggled = id.clone();
            let removed = id.clone();
            h_flex()
                .gap_2()
                .child(
                    Checkbox::new(named("done", &id))
                        .label(todo.title)
                        .checked(todo.done)
                        .on_click(cx.listener(move |this, _: &bool, _, cx| {
                            write(&this.todos, &mut this.failed, cx, |todos| {
                                todos.toggle(&toggled)
                            });
                        })),
                )
                .child(Button::new(named("remove", &id)).label("✕").on_click(cx.listener(
                    move |this, _, _, cx| {
                        write(&this.todos, &mut this.failed, cx, |todos| todos.remove(&removed));
                    },
                )))
        });

        let (done, total) = todos.tally(&self.list);

        v_flex()
            .gap_2()
            .child(div().text_xl().child(found.name))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w_64().child(Input::new(&self.draft)))
                    .child(
                        Button::new("add")
                            .label("add")
                            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                    ),
            )
            .children(rows)
            .child(
                h_flex()
                    .gap_2()
                    .child(SharedString::from(format!("{} left", total - done)))
                    .child(Button::new("clear-done").label("clear done").on_click(cx.listener(
                        |this, _, _, cx| {
                            let list = this.list.clone();
                            write(&this.todos, &mut this.failed, cx, |todos| {
                                todos.clear_done(&list)
                            });
                        },
                    ))),
            )
            .child(footer(self.heard, &self.failed))
    }
}

struct SettingsPage {
    todos: AmeEntity<Todos>,
    heard: u64,
    failed: Option<SharedString>,
    _watching: Subscription,
}

impl SettingsPage {
    fn new(todos: AmeEntity<Todos>, cx: &mut Context<Self>) -> Self {
        let _watching = cx.observe(&todos, |this, _, cx| {
            this.heard += 1;
            cx.notify();
        });

        Self {
            todos,
            heard: 0,
            failed: None,
            _watching,
        }
    }
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hide_done = held(&self.todos, cx).hide_done().get();

        v_flex()
            .gap_2()
            .child(div().text_xl().child("settings"))
            .child(
                Checkbox::new("hide-done")
                    .label("hide done")
                    .checked(hide_done)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        let hide = *checked;
                        write(&this.todos, &mut this.failed, cx, |todos| {
                            Ok(todos.hide_done().set(hide)?)
                        });
                    })),
            )
            .child(footer(self.heard, &self.failed))
    }
}

struct Shell {
    todos: AmeEntity<Todos>,
    body: AnyView,
}

impl Shell {
    fn new(todos: AmeEntity<Todos>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = Self::open(Page::Overview, &todos, cx.weak_entity(), window, cx);
        Self { todos, body }
    }

    fn open(
        page: Page,
        todos: &AmeEntity<Todos>,
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        match page {
            Page::Overview => cx
                .new(|cx| Overview::new(todos.clone(), shell, window, cx))
                .into(),
            Page::List(list) => cx
                .new(|cx| ListPage::new(todos.clone(), list, window, cx))
                .into(),
            Page::Settings => cx.new(|cx| SettingsPage::new(todos.clone(), cx)).into(),
        }
    }

    fn go(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.body = Self::open(page, &self.todos, cx.weak_entity(), window, cx);
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("nav-lists").label("lists").on_click(cx.listener(
                        |this, _, window, cx| this.go(Page::Overview, window, cx),
                    )))
                    .child(Button::new("nav-settings").label("settings").on_click(cx.listener(
                        |this, _, window, cx| this.go(Page::Settings, window, cx),
                    ))),
            )
            .child(self.body.clone())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let store = StoreBuilder::located(|at| at.app("amethystate-examples", "gpui-todo"))?
        .backend(Backend::Json)
        .build()?;

    gpui_platform::application()
        .with_assets(gpui_kit_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);

            let todos: AmeEntity<Todos> = cx.new_amethystate(|| Todos::new_with(&store));

            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("todos - amethystate + gpui".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                let shell = cx.new(|cx| Shell::new(todos, window, cx));
                cx.new(|cx| Root::new(shell, window, cx))
            })
            .expect("the window would not open");
        });

    Ok(())
}
