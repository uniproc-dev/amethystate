use amethystate::observability::InspectorBackend;
use amethystate::store::meta::SchemaSnapshot;

pub enum ViewMode {
    All,
    Flatten,
    Struct(String),
}

pub struct App {
    pub backend: Box<dyn InspectorBackend>,
    pub mode: ViewMode,
    pub structs: Vec<(String, SchemaSnapshot)>,
    pub selected: usize,
}

impl App {
    pub fn new(backend: Box<dyn InspectorBackend>) -> anyhow::Result<Self> {
        let structs = backend
            .get_schema_snapshots()
            .map_err(crate::report::anyhowed)?;

        Ok(Self {
            backend,
            mode: ViewMode::All,
            structs,
            selected: 0,
        })
    }

    pub fn sidebar_items_count(&self) -> usize {
        self.structs.len() + 3
    }

    pub fn select_next(&mut self) {
        let len = self.sidebar_items_count();
        self.selected = (skip_board(self.selected, Skip::Plus)) % len;
        self.select_mode();
    }

    pub fn select_mode(&mut self) {
        match self.selected {
            0 => self.mode = ViewMode::All,
            1 => self.mode = ViewMode::Flatten,
            2.. => self.mode = ViewMode::Struct(self.structs[self.selected - 3].0.clone()),
        }
    }
    pub fn select_prev(&mut self) {
        let len = self.sidebar_items_count();
        if self.selected == 0 {
            self.selected = len - 1;
        } else {
            self.selected = skip_board(self.selected, Skip::Minus);
        }
        self.select_mode();
    }
}

enum Skip {
    Plus,
    Minus,
}
fn skip_board(selected: usize, skip: Skip) -> usize {
    match skip {
        Skip::Plus => {
            if selected == 1 {
                selected + 2
            } else {
                selected + 1
            }
        }
        Skip::Minus => {
            if selected == 3 {
                selected - 2
            } else {
                selected - 1
            }
        }
    }
}
