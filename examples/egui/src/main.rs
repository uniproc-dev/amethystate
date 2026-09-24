use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use eframe::egui;
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

struct TodoApp {
    todos: Todos,
    page: Page,
    draft: String,
    failed: Option<String>,
}

impl TodoApp {
    fn report(&mut self, done: Done) {
        if let Err(why) = done {
            self.failed = Some(why.to_string());
        }
    }

    fn overview(&mut self, ui: &mut egui::Ui) {
        ui.heading("lists");

        for (id, list) in self.todos.all_lists() {
            let (done, total) = self.todos.tally(&id);
            ui.horizontal(|ui| {
                if ui.link(&list.name).clicked() {
                    self.page = Page::List(id.clone());
                }
                ui.label(format!("{done}/{total}"));
                if ui.small_button("✕").clicked() {
                    let removed = self.todos.remove_list(&id);
                    self.report(removed);
                }
            });
        }

        ui.horizontal(|ui| {
            let field = ui.text_edit_singleline(&mut self.draft);
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("add list").clicked() || entered {
                let added = self.todos.add_list(&self.draft);
                self.report(added);
                self.draft.clear();
                field.request_focus();
            }
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(list) = self.todos.lists().get(id) else {
            ui.label("this list is gone");
            return;
        };

        ui.heading(&list.name);

        ui.horizontal(|ui| {
            let field = ui.text_edit_singleline(&mut self.draft);
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("add").clicked() || entered {
                let added = self.todos.add(id, &self.draft);
                self.report(added);
                self.draft.clear();
                field.request_focus();
            }
        });

        for (item, todo) in self.todos.shown_in(id) {
            ui.horizontal(|ui| {
                let mut done = todo.done;
                if ui.checkbox(&mut done, &todo.title).changed() {
                    let toggled = self.todos.toggle(&item);
                    self.report(toggled);
                }
                if ui.small_button("✕").clicked() {
                    let removed = self.todos.remove(&item);
                    self.report(removed);
                }
            });
        }

        ui.separator();

        let (done, total) = self.todos.tally(id);
        ui.horizontal(|ui| {
            ui.label(format!("{} left", total - done));
            if ui.button("clear done").clicked() {
                let cleared = self.todos.clear_done(id);
                self.report(cleared);
            }
        });
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("settings");

        let mut hide_done = self.todos.hide_done().get();
        if ui.checkbox(&mut hide_done, "hide done").changed() {
            let hidden = self.todos.hide_done().set(hide_done).map_err(Into::into);
            self.report(hidden);
        }
    }
}

impl eframe::App for TodoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.page == Page::Overview, "lists")
                .clicked()
            {
                self.page = Page::Overview;
            }
            if ui
                .selectable_label(self.page == Page::Settings, "settings")
                .clicked()
            {
                self.page = Page::Settings;
            }
        });
        ui.separator();

        match self.page.clone() {
            Page::Overview => self.overview(ui),
            Page::List(id) => self.list(ui, &id),
            Page::Settings => self.settings(ui),
        }

        if let Some(why) = &self.failed {
            ui.colored_label(egui::Color32::RED, why);
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let store =
        StoreBuilder::located(|at| at.app("amethystate-examples", "egui-todo"))?.build()?;
    let todos = Todos::new_with(&store)?;

    eframe::run_native(
        "todos",
        eframe::NativeOptions::default(),
        Box::new(move |_| {
            Ok(Box::new(TodoApp {
                todos,
                page: Page::Overview,
                draft: String::new(),
                failed: None,
            }))
        }),
    )?;

    Ok(())
}
