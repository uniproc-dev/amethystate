mod primitives;

#[cfg(not(target_arch = "wasm32"))]
mod native;

mod framework;
#[cfg(target_arch = "wasm32")]
mod wasm;

pub use framework::*;

pub use primitives::*;

#[cfg(not(target_arch = "wasm32"))]
pub use native::Arena;

#[cfg(target_arch = "wasm32")]
pub use wasm::Arena;

#[cfg(not(target_arch = "wasm32"))]
pub type DefaultArena = Arena;

#[cfg(target_arch = "wasm32")]
#[cfg(feature = "tauri-backend")]
pub type DefaultArena = Arena<amethystate_tauri::TauriBackend>;

#[cfg(all(target_arch = "wasm32", not(feature = "tauri-backend")))]
compile_error!(
    "The 'tauri-backend' feature must be enabled when compiling for the 'wasm32' target."
);
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
