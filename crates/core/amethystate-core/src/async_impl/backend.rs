use crate::path::StorePath;
use crate::primitives::field_core::FieldValue;
use crate::primitives::map_core::{ReactiveMapKey, ReactiveMapValue};
use crate::{AmeBackendAsync, FieldCore, ReactiveMapCore};
use serde::Deserialize;

pub struct SubscriptionHandle {
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl SubscriptionHandle {
    pub fn new(cleanup: impl FnOnce() + Send + 'static) -> Self {
        Self {
            cleanup: Some(Box::new(cleanup)),
        }
    }

    pub fn noop() -> Self {
        Self { cleanup: None }
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

pub trait AsyncSubscriptionBackend: AmeBackendAsync + Clone + Send + Sync + 'static {
    fn subscribe_field<T>(&self, path: StorePath, core: FieldCore<T>) -> SubscriptionHandle
    where
        T: FieldValue;

    fn subscribe_map<K, V>(
        &self,
        path: StorePath,
        core: ReactiveMapCore<K, V>,
    ) -> SubscriptionHandle
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue;
}
