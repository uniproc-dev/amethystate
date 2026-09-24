use amethystate::store::builder::Backend;
use amethystate::{ReactiveMap, StoreBuilder, amethystate};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::time::Duration;

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

struct App {
    todos: Todos,
    page: Page,
    draft: Option<String>,
    selected: ListState,
    failed: Option<String>,
    quit: bool,
}

impl App {
    fn report(&mut self, done: Done) {
        if let Err(why) = done {
            self.failed = Some(why.to_string());
        }
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.draft = None;
        self.selected = ListState::default();
    }

    fn ids(&self) -> Vec<String> {
        match &self.page {
            Page::Overview => self.todos.all_lists().into_iter().map(|(id, _)| id).collect(),
            Page::List(list) => self.todos.shown_in(list).into_iter().map(|(id, _)| id).collect(),
            Page::Settings => Vec::new(),
        }
    }

    fn picked(&self) -> Option<String> {
        self.selected.selected().and_then(|at| self.ids().into_iter().nth(at))
    }

    fn keep_in(&mut self, rows: usize) {
        match self.selected.selected() {
            _ if rows == 0 => self.selected.select(None),
            Some(at) if at >= rows => self.selected.select(Some(rows - 1)),
            None => self.selected.select(Some(0)),
            Some(_) => {}
        }
    }

    fn typing(&mut self, code: KeyCode, mut draft: String) {
        match code {
            KeyCode::Enter => {
                let added = match &self.page {
                    Page::Overview => self.todos.add_list(&draft),
                    Page::List(list) => self.todos.add(list, &draft),
                    Page::Settings => Ok(()),
                };
                self.report(added);
            }
            KeyCode::Esc => {}
            KeyCode::Backspace => {
                draft.pop();
                self.draft = Some(draft);
            }
            KeyCode::Char(c) => {
                draft.push(c);
                self.draft = Some(draft);
            }
            _ => self.draft = Some(draft),
        }
    }

    fn key(&mut self, code: KeyCode) {
        if let Some(draft) = self.draft.take() {
            return self.typing(code, draft);
        }

        match (code, self.page.clone()) {
            (KeyCode::Char('q'), _) => self.quit = true,
            (KeyCode::Char('1'), _) => self.go(Page::Overview),
            (KeyCode::Char('2'), _) => self.go(Page::Settings),
            (KeyCode::Up, _) => self.selected.select_previous(),
            (KeyCode::Down, _) => self.selected.select_next(),
            (KeyCode::Char('a'), Page::Overview | Page::List(_)) => self.draft = Some(String::new()),
            (KeyCode::Enter, Page::Overview) => {
                if let Some(list) = self.picked() {
                    self.go(Page::List(list));
                }
            }
            (KeyCode::Char('d'), Page::Overview) => {
                if let Some(list) = self.picked() {
                    let removed = self.todos.remove_list(&list);
                    self.report(removed);
                }
            }
            (KeyCode::Esc, Page::List(_)) => self.go(Page::Overview),
            (KeyCode::Char(' '), Page::List(_)) => {
                if let Some(item) = self.picked() {
                    let toggled = self.todos.toggle(&item);
                    self.report(toggled);
                }
            }
            (KeyCode::Char('d'), Page::List(_)) => {
                if let Some(item) = self.picked() {
                    let removed = self.todos.remove(&item);
                    self.report(removed);
                }
            }
            (KeyCode::Char('c'), Page::List(list)) => {
                let cleared = self.todos.clear_done(&list);
                self.report(cleared);
            }
            (KeyCode::Char(' ') | KeyCode::Char('h'), Page::Settings) => {
                let hide = !self.todos.hide_done().get();
                let hidden = self.todos.hide_done().set(hide).map_err(Into::into);
                self.report(hidden);
            }
            _ => {}
        }
    }

    fn overview(&mut self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let rows: Vec<ListItem> = self
            .todos
            .all_lists()
            .into_iter()
            .map(|(id, list)| {
                let (done, total) = self.todos.tally(&id);
                ListItem::new(format!("{}  {done}/{total}", list.name))
            })
            .collect();
        self.keep_in(rows.len());

        let list = List::new(rows)
            .block(Block::bordered().title(" lists "))
            .highlight_symbol("> ")
            .highlight_style(Style::new().fg(Color::Yellow));
        frame.render_stateful_widget(list, area, &mut self.selected);
    }

    fn list(&mut self, frame: &mut Frame, area: ratatui::layout::Rect, id: &str) {
        let Some(found) = self.todos.lists().get(id) else {
            self.keep_in(0);
            frame.render_widget(
                Paragraph::new("this list is gone").block(Block::bordered()),
                area,
            );
            return;
        };

        let rows: Vec<ListItem> = self
            .todos
            .shown_in(id)
            .into_iter()
            .map(|(_, todo)| {
                let mark = if todo.done { "[x]" } else { "[ ]" };
                ListItem::new(format!("{mark} {}", todo.title))
            })
            .collect();
        self.keep_in(rows.len());

        let (done, total) = self.todos.tally(id);
        let list = List::new(rows)
            .block(Block::bordered().title(format!(" {} - {} left ", found.name, total - done)))
            .highlight_symbol("> ")
            .highlight_style(Style::new().fg(Color::Yellow));
        frame.render_stateful_widget(list, area, &mut self.selected);
    }

    fn settings(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let mark = if self.todos.hide_done().get() { "[x]" } else { "[ ]" };
        frame.render_widget(
            Paragraph::new(format!("{mark} hide done")).block(Block::bordered().title(" settings ")),
            area,
        );
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [tabs, body, draft, help, failed] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let tab = |label: &'static str, on: bool| {
            if on {
                Span::from(label).reversed()
            } else {
                Span::from(label)
            }
        };
        let on_settings = self.page == Page::Settings;
        frame.render_widget(
            Line::from(vec![
                tab(" 1 lists ", !on_settings),
                Span::from(" "),
                tab(" 2 settings ", on_settings),
            ]),
            tabs,
        );

        match self.page.clone() {
            Page::Overview => self.overview(frame, body),
            Page::List(id) => self.list(frame, body, &id),
            Page::Settings => self.settings(frame, body),
        }

        if let Some(text) = &self.draft {
            frame.render_widget(Line::from(format!("new: {text}_")).yellow(), draft);
        }

        let keys = match (&self.page, self.draft.is_some()) {
            (_, true) => "enter add  esc cancel",
            (Page::Overview, _) => "up/down pick  enter open  a add  d remove  q quit",
            (Page::List(_), _) => "up/down pick  space toggle  a add  d remove  c clear done  esc back",
            (Page::Settings, _) => "space toggle  q quit",
        };
        frame.render_widget(Line::from(keys).dark_gray(), help);

        if let Some(why) = &self.failed {
            frame.render_widget(Line::from(why.as_str()).red(), failed);
        }
    }
}

fn run(terminal: &mut DefaultTerminal, todos: Todos) -> std::io::Result<()> {
    let mut app = App {
        todos,
        page: Page::Overview,
        draft: None,
        selected: ListState::default(),
        failed: None,
        quit: false,
    };

    while !app.quit {
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.key(key.code);
        }
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let store = StoreBuilder::located(|at| at.app("amethystate-examples", "ratatui-todo"))?
        .backend(Backend::Json)
        .build()?;
    let todos = Todos::new_with(&store)?;

    let mut terminal = ratatui::init();
    let ran = run(&mut terminal, todos);
    ratatui::restore();
    ran?;

    Ok(())
}
