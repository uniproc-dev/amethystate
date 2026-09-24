use amethystate::{AmeStateSlice, ReactiveScope};
use futures::StreamExt as _;
use gpui::{App, AppContext, Entity};
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone)]
pub struct AmeView<T> {
    inner: T,
    _scope: Arc<ReactiveScope>,
}

impl<T> Deref for AmeView<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: AmeStateSlice> AmeView<T> {
    pub fn new(inner: T, tx: futures::channel::mpsc::UnboundedSender<()>) -> Self {
        let _scope = inner.subscribe_all_external(move || {
            let _ = tx.unbounded_send(());
        });

        Self {
            inner,
            _scope: Arc::new(_scope),
        }
    }
}

pub type AmeEntity<T> = Entity<AmeView<T>>;

pub trait AmeStateExt {
    /// An entity over the slice `f` opens, re-rendered on every external change to it.
    ///
    /// Panics with the error when `f` fails; [`AmeStateExt::try_new_amethystate`] returns it.
    fn new_amethystate<T: AmeStateSlice + 'static, E: Debug>(
        &mut self,
        f: impl FnOnce() -> Result<T, E>,
    ) -> AmeEntity<T>;

    /// [`AmeStateExt::new_amethystate`] for a slice that can fail to open.
    fn try_new_amethystate<T: AmeStateSlice + 'static, E>(
        &mut self,
        f: impl FnOnce() -> Result<T, E>,
    ) -> Result<AmeEntity<T>, E>;
}

impl AmeStateExt for App {
    #[track_caller]
    fn new_amethystate<T: AmeStateSlice + 'static, E: Debug>(
        &mut self,
        f: impl FnOnce() -> Result<T, E>,
    ) -> AmeEntity<T> {
        self.try_new_amethystate(f)
            .unwrap_or_else(|error| panic!("amethystate: the state would not open: {error:?}"))
    }

    fn try_new_amethystate<T: AmeStateSlice + 'static, E>(
        &mut self,
        f: impl FnOnce() -> Result<T, E>,
    ) -> Result<AmeEntity<T>, E> {
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

            AmeView::new(inner, tx)
        });

        Ok(entity)
    }
}
