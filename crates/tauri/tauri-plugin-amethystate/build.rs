const COMMANDS: &[&str] = &[
    "amethystate_get",
    "amethystate_set",
    "amethystate_delete",
    "amethystate_delete_prefix",
    "amethystate_scan_keys",
    "amethystate_subscribe",
    "amethystate_unsubscribe",
    "amethystate_get_prefix",
    "amethystate_flush",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
