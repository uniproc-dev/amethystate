use amethystate_macros::amethystate;

#[amethystate(prefix = "editor", target = "tauri-wasm", mode = "persistent")]
pub struct Editor {
    #[amestate(default = 1280)]
    pub width: u32,
}

fn main() {}
