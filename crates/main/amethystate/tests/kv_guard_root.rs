use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod common;

#[amethystate(as_root)]
pub struct RootConfig {
    #[amestate(default = 1280u32)]
    pub width: u32,
}

#[amethystate(prefix = "guarded")]
pub struct GuardedConfig {
    #[amestate(default = 1280u32)]
    pub width: u32,
}

/// Control: `Kv` refuses a path a prefixed struct owns.
#[backends(all)]
fn kv_refuses_a_path_owned_by_a_prefixed_struct(backend: Backend) {
    let path = TempPath::new("kv_guard_prefixed");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let _cfg = GuardedConfig::new_with(&store).unwrap();

    let err = store
        .kv()
        .namespace("guarded")
        .set("width", &"oops".to_string())
        .unwrap_err();

    insta::assert_snapshot!("kv_write_under_a_declared_prefix", err.to_string());
}

/// A root struct's fields sit at bare paths, one level below nothing. Ownership
/// is by declared path rather than by prefix, so those paths are declared like
/// any others and a `Kv` write is refused there too.
#[backends(all)]
fn kv_refuses_a_path_owned_by_a_root_struct(backend: Backend) {
    let path = TempPath::new("kv_guard_root");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let cfg = RootConfig::new_with(&store).unwrap();

    let err = store.kv().set("width", &"oops".to_string()).unwrap_err();

    insta::assert_snapshot!("kv_write_over_a_root_struct_field", err.to_string());

    assert_eq!(cfg.width().get(), 1280);
}

/// What the refusal buys: a path written over with a type it cannot decode
/// stops the struct loading on the next run, so the write has to be refused
/// while the store is still open rather than found after a restart.
#[backends(all)]
fn a_root_field_survives_a_kv_write_of_another_type(backend: Backend) {
    let path = TempPath::new("kv_guard_root_reopen");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let _cfg = RootConfig::new_with(&store).unwrap();
        store.kv().set("width", &"oops".to_string()).ok();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let cfg = RootConfig::new_with(&store);
    assert!(cfg.is_ok(), "the struct still loads after a reopen");
}
