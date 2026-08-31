use crate::Store;
use crate::store::error::StorageResult;
use amethystate_core::ReactiveScope;
use amethystate_core::path::StorePath;

/// Where a declared struct's fields live, as the levels they are under.
pub trait StateScope {
    const PATH: StorePath;

    /// `PATH` as one string, for the places that need it before the program
    /// runs - a migration's dependency list, which is a `&'static [&'static
    /// str]`. The macro writes this and the joined half of `PATH` from the same
    /// token, so the two cannot come apart.
    const KEY: &'static str;
}

pub trait AmeStateSlice: Sized {
    fn load_slice(store: &Store) -> StorageResult<Self>;

    fn subscribe_all<F>(&self, callback: F) -> ReactiveScope
    where
        F: Fn() + Send + Sync + 'static;

    fn subscribe_all_external<F>(&self, callback: F) -> ReactiveScope
    where
        F: Fn() + Send + Sync + 'static;
}
