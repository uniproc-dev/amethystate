use crate::async_impl::{AsyncSubscriptionBackend, SubscriptionHandle};
use crate::error::FieldError;
use crate::facts::Facts;
use crate::path::StorePath;
use crate::primitives::error::ReactiveFieldResult;
use crate::primitives::field_core::FieldValue;
use crate::{Change, FieldCore, InterceptDisposer, Signal, SignalSubscription};
use error_stack::{Report, ResultExt};
use std::fmt::{self, Debug};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub struct Field<T, B> {
    pub core: FieldCore<T>,
    pub path: StorePath,
    pub instance_id: Uuid,
    _subscription: Arc<Mutex<SubscriptionHandle>>,
    backend: B,
}

impl<T, B> Clone for Field<T, B>
where
    B: Clone,
{
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            path: self.path.clone(),
            instance_id: self.instance_id,
            _subscription: self._subscription.clone(),
            backend: self.backend.clone(),
        }
    }
}

impl<T, B> PartialEq for Field<T, B> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.instance_id == other.instance_id
            && Signal::ptr_eq(&self.core.signal, &other.core.signal)
    }
}

impl<T, B> Eq for Field<T, B> {}

impl<T, B> Debug for Field<T, B>
where
    T: Debug + Clone + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Field")
            .field("path", &self.path)
            .field("value", &self.core.get())
            .finish()
    }
}

impl<T, B> Field<T, B>
where
    T: FieldValue,
    B: AsyncSubscriptionBackend,
{
    pub fn fork(&self) -> Self {
        self.fork_with_id(Uuid::new_v4())
    }

    pub fn fork_with_id(&self, new_instance_id: Uuid) -> Self {
        Self {
            core: self.core.clone(),
            path: self.path.clone(),
            instance_id: new_instance_id,
            _subscription: self._subscription.clone(),
            backend: self.backend.clone(),
        }
    }

    pub fn new(key: StorePath, initial_value: T) -> Self
    where
        B: Default,
    {
        Self::new_with_backend(key, initial_value, B::default())
    }

    pub fn new_with_backend(key: StorePath, initial_value: T, backend: B) -> Self {
        Self::new_with_backend_and_id(key, initial_value, backend, Uuid::new_v4())
    }

    pub fn new_with_backend_and_id(
        key: StorePath,
        initial_value: T,
        backend: B,
        instance_id: Uuid,
    ) -> Self {
        let path = key;
        let core = FieldCore::new(initial_value);
        let subscription = backend.subscribe_field(path.clone(), core.clone());

        Self {
            core,
            path,
            instance_id,
            _subscription: Arc::new(Mutex::new(subscription)),
            backend,
        }
    }

    pub fn value(&self) -> T {
        self.core.get()
    }

    pub async fn get(&self) -> ReactiveFieldResult<T> {
        self.backend
            .get(&self.path)
            .await
            .change_context(FieldError::Storage)
            .attach_key(&self.path)?
            .ok_or_else(|| {
                Report::new(FieldError::KeyNotFound(self.path.to_string()))
                    .attach("read through a field handle")
            })
    }

    pub async fn update<F>(&self, f: F) -> ReactiveFieldResult<T>
    where
        F: FnOnce(T) -> T,
    {
        let val = self.get().await?;
        let new_val = f(val);
        self.set(new_val.clone()).await?;
        Ok(new_val)
    }

    pub async fn modify<F>(&self, f: F) -> ReactiveFieldResult<()>
    where
        F: FnOnce(&mut T),
    {
        let mut val = self.get().await?;
        f(&mut val);
        self.set(val).await
    }

    pub async fn set(&self, value: T) -> ReactiveFieldResult<()> {
        crate::field_set_async(
            &self.backend,
            &self.core,
            self.path.clone(),
            value,
            Some(self.instance_id),
        )
        .await
    }

    pub fn subscribe<F>(&self, callback: F) -> SignalSubscription
    where
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.core.subscribe(callback)
    }

    pub fn subscribe_external<F>(&self, callback: F) -> SignalSubscription
    where
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        let my_id = self.instance_id;
        self.core.subscribe_with_source(move |val, src| {
            if src != Some(my_id) {
                callback(val);
            }
        })
    }

    pub fn intercept<F>(&self, callback: F) -> InterceptDisposer
    where
        F: Fn(Change<T>) -> Option<Change<T>> + Send + Sync + 'static,
    {
        self.core.intercept(self.path.clone(), callback)
    }
}
