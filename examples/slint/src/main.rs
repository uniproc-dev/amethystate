use amethystate::store::builder::Backend;
use amethystate::{ReactiveMap, ReactiveScope, SignalSubscription, StoreBuilder, amethystate};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

slint::include_modules!();

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

fn refresh(ui: &AppWindow, todos: &Todos) {
    let lists: Vec<ListRow> = todos
        .all_lists()
        .into_iter()
        .map(|(id, list)| {
            let (done, total) = todos.tally(&id);
            ListRow {
                id: id.into(),
                name: list.name.into(),
                tally: format!("{done}/{total}").into(),
            }
        })
        .collect();
    ui.set_lists(ModelRc::new(VecModel::from(lists)));

    let list = ui.get_current_list().to_string();
    match todos.lists().get(&list) {
        Some(found) => {
            ui.set_list_name(found.name.into());
            ui.set_list_gone(false);
        }
        None => ui.set_list_gone(true),
    }

    let rows: Vec<TodoRow> = todos
        .shown_in(&list)
        .into_iter()
        .map(|(id, todo)| TodoRow {
            id: id.into(),
            title: todo.title.into(),
            done: todo.done,
        })
        .collect();
    ui.set_rows(ModelRc::new(VecModel::from(rows)));

    let (done, total) = todos.tally(&list);
    ui.set_left((total - done) as i32);
    ui.set_hide_done(todos.hide_done().get());
}

fn report(ui: &Weak<AppWindow>, done: Done) {
    if let (Err(why), Some(ui)) = (done, ui.upgrade()) {
        ui.set_failed(why.to_string().into());
    }
}

struct Pages {
    todos: Todos,
    ui: Weak<AppWindow>,
    scope: RefCell<ReactiveScope>,
}

impl Pages {
    fn heard(&self) -> impl Fn() + Send + Sync + 'static {
        let todos = self.todos.clone();
        let ui = self.ui.clone();
        move || {
            let todos = todos.clone();
            let _ = ui.upgrade_in_event_loop(move |ui| {
                ui.set_heard(ui.get_heard() + 1);
                refresh(&ui, &todos);
            });
        }
    }

    fn watch(&self, subs: impl IntoIterator<Item = SignalSubscription>) {
        let mut scope = ReactiveScope::new();
        for sub in subs {
            scope.watch(sub);
        }
        *self.scope.borrow_mut() = scope;

        if let Some(ui) = self.ui.upgrade() {
            ui.set_heard(0);
            refresh(&ui, &self.todos);
        }
    }

    fn overview(&self) {
        let (lists, items) = (self.heard(), self.heard());
        self.watch([
            self.todos.lists().subscribe_any(move |_| lists()),
            self.todos.items().subscribe_any(move |_| items()),
        ]);
    }

    fn list(&self, id: String) {
        let (list, items, hide_done) = (self.heard(), self.heard(), self.heard());
        self.watch([
            self.todos.lists().subscribe_key(id, move |_| list()),
            self.todos.items().subscribe_any(move |_| items()),
            self.todos.hide_done().subscribe(move |_| hide_done()),
        ]);
    }

    fn settings(&self) {
        let hide_done = self.heard();
        self.watch([self.todos.hide_done().subscribe(move |_| hide_done())]);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let store = StoreBuilder::located(|at| at.app("amethystate-examples", "slint-todo"))?
        .backend(Backend::Json)
        .build()?;
    let todos = Todos::new_with(&store)?;

    let ui = AppWindow::new()?;
    let weak = ui.as_weak();
    let pages = Rc::new(Pages {
        todos: todos.clone(),
        ui: weak.clone(),
        scope: RefCell::new(ReactiveScope::new()),
    });

    ui.on_overview_opened({
        let pages = Rc::clone(&pages);
        move || pages.overview()
    });
    ui.on_list_opened({
        let pages = Rc::clone(&pages);
        move |id| pages.list(id.to_string())
    });
    ui.on_settings_opened({
        let pages = Rc::clone(&pages);
        move || pages.settings()
    });

    ui.on_add_list({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |name| report(&weak, todos.add_list(&name))
    });
    ui.on_remove_list({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |list| report(&weak, todos.remove_list(&list))
    });
    ui.on_add({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |list, title| report(&weak, todos.add(&list, &title))
    });
    ui.on_toggle({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |id| report(&weak, todos.toggle(&id))
    });
    ui.on_remove({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |id| report(&weak, todos.remove(&id))
    });
    ui.on_clear_done({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |list| report(&weak, todos.clear_done(&list))
    });
    ui.on_set_hide_done({
        let (todos, weak) = (todos.clone(), weak.clone());
        move |hide| report(&weak, todos.hide_done().set(hide).map_err(Into::into))
    });

    pages.overview();
    ui.run()?;

    Ok(())
}
