use amethystate::amethystate;
use amethystate_codegen::{CodegenRegistry, TauriVanillaCodegen};

#[amethystate]
pub struct TestNested {
    #[amestate(default = "nested_val".to_string())]
    pub name: String,
}

#[amethystate(prefix = "test_root")]
pub struct TestRoot {
    #[amestate(default = 42)]
    pub value: i32,

    #[amestate(default = "volatile_val".to_string(), volatile)]
    pub session: String,

    #[amestate(nested)]
    pub child: TestNested,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tab {
    pub title: String,
}

#[amethystate(prefix = "editor")]
pub struct Editor {
    #[amestate(default = Tab::default())]
    pub pinned: Tab,

    #[amestate(default = None)]
    pub last_closed: Option<Tab>,

    #[amestate(default = {})]
    pub open_tabs: amethystate::ReactiveMap<String, Tab>,
}

#[amethystate(prefix = "renamed", rename_all = "kebab-case")]
pub struct Renamed {
    #[amestate(default = false)]
    pub dark_mode: bool,

    #[amestate(default = 1, path = "window.width")]
    pub width: u32,

    #[amestate(nested, flatten)]
    pub inner: TestNested,
}

#[test]
fn a_field_is_exported_under_the_path_it_is_stored_at() {
    let renamed = amethystate::tauri::exports()
        .iter()
        .find(|entry| entry.struct_name == "Renamed")
        .expect("Renamed was not registered");

    let stored: Vec<(&str, &str)> = renamed
        .fields
        .iter()
        .map(|field| (field.name, field.stored))
        .collect();

    assert_eq!(
        stored,
        [
            ("dark_mode", "dark-mode"),
            ("width", "window.width"),
            ("inner", "")
        ]
    );
}

#[test]
fn test_rust_codegen_export() {
    let out_path = std::env::temp_dir().join("amethystate_test_export.rs");
    if out_path.exists() {
        let _ = std::fs::remove_file(&out_path);
    }

    let reg = CodegenRegistry::new();
    reg.export_rust(&out_path, &TauriVanillaCodegen)
        .expect("Failed to export Rust bindings");

    assert!(out_path.exists());
    let rust_content =
        std::fs::read_to_string(&out_path).expect("Failed to read exported Rust file");

    insta::assert_snapshot!("rust_codegen_export", rust_content);
}

#[test]
fn test_schema_inventory_registrations() {
    let mut found_root = false;
    let mut found_nested = false;

    for entry in amethystate::tauri::exports() {
        if entry.struct_name == "TestRoot" {
            found_root = true;
            assert_eq!(entry.prefix, Some("test_root"));
            assert_eq!(entry.fields.len(), 3);
            assert_eq!(entry.fields[0].name, "value");
            assert_eq!(entry.fields[1].name, "session");
            assert_eq!(entry.fields[2].name, "child");
        } else if entry.struct_name == "TestNested" {
            found_nested = true;
            assert_eq!(entry.prefix, None);
            assert_eq!(entry.fields.len(), 1);
            assert_eq!(entry.fields[0].name, "name");
        }
    }

    assert!(found_root, "TestRoot was not registered in inventory!");
    assert!(found_nested, "TestNested was not registered in inventory!");
}

#[test]
fn test_typescript_codegen_export() {
    let out_path = std::env::temp_dir().join("amethystate_test_export.ts");
    if out_path.exists() {
        let _ = std::fs::remove_file(&out_path);
    }

    let reg = CodegenRegistry::new();
    reg.export_ts(&out_path)
        .expect("Failed to export TS bindings");

    assert!(out_path.exists());
    let ts_content = std::fs::read_to_string(&out_path).expect("Failed to read exported TS file");

    insta::assert_snapshot!("typescript_codegen_export", ts_content);
}
