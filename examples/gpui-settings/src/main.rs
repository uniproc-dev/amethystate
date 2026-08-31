use gpui::{div, prelude::*, AppContext, ParentElement, Render, Window, WindowOptions};
use gpui_component::button::Button;
use gpui_component::Root;
use amethystate::{amethystate, IntoGlobalStore, StoreBuilder};
use amethystate_gpui::{RpEntity, amethystateExt};
use std::time::Duration;

#[amethystate(prefix = "counter")]
pub struct CounterState {
    #[amestate(default = 0)]
    pub count: i32,
}

struct CounterView {
    state: RpEntity<CounterState>,
}

impl CounterView {
    fn new(cx: &mut Context<Self>) -> Self {

        let state = cx.new_amethystate(CounterState::new).unwrap();

        let forked_state = state.read(cx).fork();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let current = forked_state.count().get();
                forked_state.count().set(current + 1).ok();
            }
        });

        Self { state }
    }
}

impl Render for CounterView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();

        let current_count = state.read(cx).count().get();

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(gpui::rgb(0x1e1e2e))
            .text_color(gpui::rgb(0xcdd6f4))
            .size_full()
            .child(format!("Count: {}", current_count))
            .child(Button::new("Increment Locally")
                .bg(gpui::rgb(0x89b4fa))
                .text_color(gpui::rgb(0x11111b))
                .px_4()
                .py_2()
                .rounded_md()
                .on_click(move |_, _, cx| {
                    state.read(cx).count().update(|v| v + 1).ok();
                })
            )
    }
}

fn main() {
    let (_report, _ame) = StoreBuilder::new("./app_data.redb").init_global_with_migration();

    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(CounterView::new);

                cx.new(|cx| Root::new(view, window, cx))
            })
                .expect("Failed to open window");
        })
            .detach();
    });
}