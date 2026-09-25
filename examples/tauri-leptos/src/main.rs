mod app;
mod bindings;

use app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Debug);

    mount_to_body(|| view! { <App /> })
}
