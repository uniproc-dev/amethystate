mod primitives;

#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
mod local;

mod framework;
#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
mod wasm;

pub use framework::*;

pub use primitives::*;

#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
pub use local::Arena;

#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
pub use wasm::Arena;

#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
pub type DefaultArena = Arena;

#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
pub type DefaultArena = Arena<amethystate_tauri::TauriBackend>;

pub use amethystate_macros_arena::amethystate_framework_arena;

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::DefaultKey;
    use std::marker::PhantomData;

    fn unique_temp_dir() -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("amethystate_arena_panic_test_{nanos}"))
    }

    #[test]
    #[should_panic(expected = "Attempted to access a dropped Field")]
    fn test_dropped_field_panic() {
        let arena = DefaultArena::default();
        let fake_handle: FieldHandle<i32> = FieldHandle {
            key: DefaultKey::default(),
            _marker: PhantomData,
        };
        arena.get_field(fake_handle);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[should_panic(expected = "Type mismatch for Field")]
    fn test_field_type_mismatch_panic() {
        use amethystate::{Field, StoreBuilder};
        let temp_dir = unique_temp_dir();
        let store = StoreBuilder::new(&temp_dir).build().unwrap();
        let field: Field<i32> = amethystate::store::field_with_path(
            &store,
            ["test", "int_field"],
            42,
            uuid::Uuid::new_v4(),
        )
        .unwrap();

        let arena = Arena::new();
        let handle = arena.register_field(field);

        let bad_handle: FieldHandle<String> = FieldHandle {
            key: handle.key,
            _marker: PhantomData,
        };

        arena.get_field(bad_handle);
    }
}
