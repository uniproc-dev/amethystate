mod hooks;
pub use hooks::*;

use amethystate::{ReactiveMapKey, ReactiveMapValue};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use yew::prelude::*;

#[derive(Clone)]
pub struct MapSignal<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    pub entries: Vec<(K, V)>,
    pub set: Callback<(K, V)>,
    pub insert: Callback<(K, V)>,
    pub remove: Callback<K>,
    pub clear: Callback<()>,
}

impl<K, V> MapSignal<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    pub fn set(&self, key: K, value: V) {
        self.set.emit((key, value));
    }
    pub fn insert(&self, key: K, value: V) {
        self.insert.emit((key, value));
    }
    pub fn remove(&self, key: K) {
        self.remove.emit(key);
    }
    pub fn clear(&self) {
        self.clear.emit(());
    }
}

pub type InitFn<B> = Rc<dyn Fn(B) -> Pin<Box<dyn Future<Output = Rc<dyn Fn(Html) -> Html>>>>>;

#[derive(Properties)]
pub struct AmeStateProviderProps<B: PartialEq + Clone + 'static> {
    pub backend: B,
    pub init: InitFn<B>,
    pub fallback: Html,
    #[prop_or_default]
    pub children: Html,
}

impl<B> PartialEq for AmeStateProviderProps<B>
where
    B: Clone + PartialEq + 'static,
{
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend && Rc::ptr_eq(&self.init, &other.init)
    }
}

#[function_component(AmeStateProvider)]
pub fn ame_state_provider<B>(props: &AmeStateProviderProps<B>) -> Html
where
    B: Clone + PartialEq + 'static,
{
    let context_wrapper = use_state(|| None::<Rc<dyn Fn(Html) -> Html>>);

    {
        let context_wrapper = context_wrapper.clone();
        let init = props.init.clone();
        let backend = props.backend.clone();

        use_effect_with((), move |_| {
            wasm_bindgen_futures::spawn_local(async move {
                let wrapper_fn = init(backend).await;
                context_wrapper.set(Some(wrapper_fn));
            });
        });
    }

    match &*context_wrapper {
        Some(wrapper) => wrapper(props.children.clone()),
        None => props.fallback.clone(),
    }
}

#[macro_export]
macro_rules! preload_slices {
    ($($S:ty),+ $(,)?) => {
        {
            let init: ::std::rc::Rc<
                dyn Fn(_) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::std::rc::Rc<dyn Fn(::yew::Html) -> ::yew::Html + 'static>> + 'static>> + 'static
            > = ::std::rc::Rc::new(|backend| {
                Box::pin(async move {
                    use ::amethystate::client::AmeStateSliceAsync;

                    let mut wrapper: Box<dyn Fn(::yew::Html) -> ::yew::Html + 'static> = Box::new(|children| children);

                    $(
                        let state = <$S>::load_async(&backend)
                            .await
                            .unwrap_or_else(|e| panic!(
                                "amethystate-yew: failed to load {}: {:?}",
                                ::std::any::type_name::<$S>(),
                                e
                            ));

                        let prev_wrapper = wrapper;
                        wrapper = Box::new(move |children| {
                            let inner = prev_wrapper(children);
                            ::yew::html! {
                                <::yew::ContextProvider<$S> context={state.clone()}>
                                    { inner }
                                </::yew::ContextProvider<$S>>
                            }
                        });
                    )+

                    let res: ::std::rc::Rc<dyn Fn(::yew::Html) -> ::yew::Html + 'static> = ::std::rc::Rc::new(wrapper);
                    res
                }) as ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::std::rc::Rc<dyn Fn(::yew::Html) -> ::yew::Html + 'static>> + 'static>>
            });

            init
        }
    };
}
