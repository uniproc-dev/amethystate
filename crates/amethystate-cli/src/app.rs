use amethystate::store::InspectorBackend;
use amethystate::store::meta::SchemaSnapshot;

pub enum ViewMode {
    All,
    Flatten,

    /// Which row of [`App::structs`], rather than which prefix.
    ///
    /// A prefix is not one declaration: nothing claims a prefix, so several may
    /// sit at one as long as their places stay apart, and the store records each
    /// whole. Naming the prefix would open whichever was recorded first for
    /// every row that shares it.
    Struct(usize),
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

    /// Row 2 is the board the sidebar draws between the two modes and the
    /// structs, and `skip_board` is what steps over it. Named here as well
    /// because the arm below subtracts, and a row this asked about by any other
    /// route would take the difference below zero.
    pub fn select_mode(&mut self) {
        match self.selected {
            0 => self.mode = ViewMode::All,
            1 => self.mode = ViewMode::Flatten,
            2 => {}
            3.. => self.mode = ViewMode::Struct(self.selected - 3),
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

#[cfg(test)]
mod tests {
    use super::*;
    use amethystate::amethystate;
    use amethystate::store::builder::{Backend, StoreBuilder};
    use amethystate_core::test_utils::TempPath;

    #[amethystate(prefix = "sidebar")]
    pub struct Sidebar {
        #[amestate(default = 1u32)]
        pub width: u32,
    }

    fn opened(held: &TempPath) -> App {
        let at = held.path().with_extension("json");
        {
            let store = StoreBuilder::new(&at)
                .backend(Backend::Json)
                .build()
                .unwrap();
            Sidebar::new_with(&store).unwrap();
            store.save_now().unwrap();
        }

        App::new(crate::inspector::open_inspector(&at).unwrap()).unwrap()
    }

    #[test]
    fn walking_the_sidebar_reaches_every_row_and_no_other() {
        let held = TempPath::new("sidebar_walk");
        let mut app = opened(&held);
        let rows = app.sidebar_items_count();

        for _ in 0..rows * 3 {
            app.select_next();
            assert_ne!(app.selected, 2, "row 2 is the board and nothing selects it");
            assert!(app.selected < rows);
        }

        for _ in 0..rows * 3 {
            app.select_prev();
            assert_ne!(app.selected, 2, "row 2 is the board and nothing selects it");
            assert!(app.selected < rows);
        }
    }

    #[test]
    fn the_board_row_asked_about_directly_opens_nothing() {
        let held = TempPath::new("sidebar_board");
        let mut app = opened(&held);

        app.selected = 2;
        app.select_mode();

        assert!(matches!(app.mode, ViewMode::All));
    }
}
