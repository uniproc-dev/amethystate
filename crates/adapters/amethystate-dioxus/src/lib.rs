mod hooks;
pub use amethystate::*;
pub use amethystate_arena::*;
use dioxus::prelude::{
    Callback, Element, Props, ReadSignal, ReadableExt, dioxus_core, dioxus_signals, rsx,
    use_context_provider,
};
pub use hooks::*;

pub struct DioxusBackend;

impl ReactiveBackend for DioxusBackend {
    type Callback<T: Send + Sync + 'static> = Callback<T>;
    type ReadSignal<T: Send + Sync + 'static> = ReadSignal<T>;

    #[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
    type Storage = amethystate::Store;

    #[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
    type Storage = amethystate::tauri::TauriBackend;

    fn cb_call<T: Send + Sync + 'static>(cb: &Self::Callback<T>, val: T) {
        cb.call(val);
    }

    fn rs_get<T: Clone + Send + Sync + 'static>(rs: &Self::ReadSignal<T>) -> T {
        rs.read().clone()
    }
}

pub type MapSignal<K, V> = amethystate_arena::MapSignal<DioxusBackend, K, V>;

#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
#[derive(Clone, Props, PartialEq)]
pub struct AmeStateProviderProps {
    pub store: Store,
    pub children: Element,
}

#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
#[derive(Clone, Props)]
pub struct AmeStateProviderProps {
    pub backend: ::amethystate::tauri::TauriBackend,
    pub init: std::rc::Rc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>,
    pub children: Element,
}

#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
impl PartialEq for AmeStateProviderProps {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.init, &other.init) && self.children == other.children
    }
}

#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
#[allow(non_snake_case)]
pub fn AmeStateProvider(props: AmeStateProviderProps) -> Element {
    use_context_provider(DefaultArena::new);
    use_context_provider(|| props.store.clone());

    rsx! { {props.children} }
}

#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
#[allow(non_snake_case)]
pub fn AmeStateProvider(props: AmeStateProviderProps) -> Element {
    use_context_provider(DefaultArena::new);
    use_context_provider(|| props.backend.clone());

    let init = props.init.clone();
    let res = dioxus::prelude::use_resource(move || {
        let f = init();
        async move {
            f.await;
        }
    });

    res.suspend()?;

    rsx! { {props.children} }
}

#[macro_export]
macro_rules! preload_slices {
    ($($S:ty),+ $(,)?) => {{
        std::rc::Rc::new(|| Box::pin(async move {
            use amethystate_arena::AmeStateFrameworkNested;
            let backend = ::dioxus::prelude::use_context::<::amethystate::tauri::TauriBackend>();
            let arena = ::dioxus::prelude::use_context::<::amethystate_arena::DefaultArena>();
            $(
                {
                    use ::amethystate_core::AmeStateSliceAsync;
                    let state = <$S as AmeStateSliceAsync<::amethystate::tauri::TauriBackend>>
                        ::load_async(&backend)
                        .await
                        .unwrap_or_else(|e| panic!("amethystate: failed to load {}: {e:?}",
                            ::std::any::type_name::<$S>()));
                    let handle = state.register(&arena);
                    ::dioxus::prelude::provide_context(handle);
                }
            )+
        }) as ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ()>>>)
    }};
}
