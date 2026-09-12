use amethystate::store::StoreBackend;
use amethystate::{AmeStateSlice, ReactiveScope, Store};
use futures::StreamExt as _;
use gpui::{App, AppContext, Entity};
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone)]
pub struct RpView<T, S> {
    inner: T,
    _scope: Arc<ReactiveScope>,
    _phantom: PhantomData<S>,
}

impl<T, S> Deref for RpView<T, S> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<S: StoreBackend, T: AmeStateSlice> RpView<T, S> {
    pub fn new(inner: T, tx: futures::channel::mpsc::UnboundedSender<()>) -> Self {
        let _scope = inner.subscribe_all_external(move || {
            let _ = tx.unbounded_send(());
        });

        Self {
            inner,
            _scope: Arc::new(_scope),
            _phantom: PhantomData,
        }
    }
}

pub type RpEntity<T, S = Store> = Entity<RpView<T, S>>;

pub trait AmeStateExt {
    fn new_amethystate<S: StoreBackend, T: AmeStateSlice + 'static, E>(
        &mut self,
        f: impl FnOnce() -> Result<T, E>,
    ) -> Result<RpEntity<T, S>, E>;
}

impl AmeStateExt for App {
    fn new_amethystate<S: StoreBackend, T: AmeStateSlice + 'static, E>(
        &mut self,
        f: impl FnOnce() -> Result<T, E>,
    ) -> Result<RpEntity<T, S>, E> {
        let reservation = self.reserve_entity();
        let inner = f()?;

        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();

        let entity = self.insert_entity(reservation, move |ctx| {
            ctx.spawn(async move |this, cx| {
                while let Some(()) = rx.next().await {
                    let _ = this.update(cx, |_, entity_cx| {
                        entity_cx.notify();
                    });
                }
            })
            .detach();

            RpView::new(inner, tx)
        });

        Ok(entity)
    }
}
