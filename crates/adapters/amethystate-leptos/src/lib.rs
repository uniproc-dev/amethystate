mod hooks;
pub use amethystate::*;
pub use hooks::*;
#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
use leptos::prelude::Children;
use leptos::prelude::{Callable, Get, IntoView, ReadSignal, component, provide_context};
#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
use leptos::prelude::{ChildrenFn, LocalResource, Owner, Suspend, Suspense, ViewFnOnce, view};

use amethystate_arena::{DefaultArena, ReactiveBackend};
use leptos::callback::Callback;

pub struct LeptosBackend;

impl ReactiveBackend for LeptosBackend {
    type Callback<T: Send + Sync + 'static> = Callback<T>;
    type ReadSignal<T: Send + Sync + 'static> = ReadSignal<T>;

    #[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
    type Storage = Store;

    #[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
    type Storage = amethystate::tauri::TauriBackend;

    fn cb_call<T: Send + Sync + 'static>(cb: &Self::Callback<T>, val: T) {
        cb.run(val);
    }

    fn rs_get<T: Clone + Send + Sync + 'static>(rs: &Self::ReadSignal<T>) -> T {
        rs.get()
    }
}

pub type MapSignal<K, V> = amethystate_arena::MapSignal<LeptosBackend, K, V>;

#[cfg(not(all(target_arch = "wasm32", feature = "tauri-backend")))]
#[component]
pub fn AmeStateProvider(store: Store, children: Children) -> impl IntoView {
    provide_context(DefaultArena::new());
    provide_context(store);

    children()
}

#[cfg(all(target_arch = "wasm32", feature = "tauri-backend"))]
#[component]
pub fn AmeStateProvider(
    backend: amethystate::tauri::TauriBackend,
    init: Box<
        dyn Fn() -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = std::sync::Arc<std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>>,
                    >,
            >,
        >,
    >,
    #[prop(into)] fallback: ViewFnOnce,
    children: ChildrenFn,
) -> impl IntoView {
    provide_context(DefaultArena::new());
    provide_context(backend);

    let resource = LocalResource::new(move || (init)());

    let children = std::sync::Arc::new(children);
    let owner = Owner::current().expect("amethystate: AmeStateProvider must have an active Owner");

    view! {
        <Suspense fallback=fallback>
            {move || {
                let children_clone = children.clone();
                let owner = owner.clone();
                Suspend::new(async move {
                    let providers = resource.await;
                    owner.with(move || {
                        for provide in providers.lock().unwrap().drain(..) {
                            provide();
                        }
                        children_clone()
                    })
                })
            }}
        </Suspense>
    }
}

#[macro_export]
macro_rules! preload_slices {
    ($($S:ty),+ $(,)?) => {{
        Box::new(move || {
            Box::pin(async move {
                use ::amethystate_arena::AmeStateFrameworkNested;
                use ::amethystate::client::AmeStateSliceAsync;

                let backend = ::leptos::prelude::use_context::<::amethystate::tauri::TauriBackend>()
                    .expect("amethystate: TauriBackend not in context");
                let arena = ::leptos::prelude::use_context::<::amethystate_arena::DefaultArena>()
                    .expect("amethystate: Arena not in context");

                let providers: ::std::sync::Arc<::std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>> =
                    ::std::sync::Arc::new(::std::sync::Mutex::new(Vec::new()));

                $(
                    {
                        let state = <$S as AmeStateSliceAsync<::amethystate::tauri::TauriBackend>>
                            ::load_async(&backend)
                            .await
                            .unwrap_or_else(|e| panic!(
                                "amethystate: failed to load {}: {e:?}",
                                ::std::any::type_name::<$S>()
                            ));
                        let handle = state.register(&arena);
                        providers.lock().unwrap().push(Box::new(move || {
                            ::leptos::prelude::provide_context(handle);
                        }));
                    }
                )+

                providers
            }) as ::std::pin::Pin<Box<dyn ::std::future::Future<Output =
                ::std::sync::Arc<::std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>>>>>
        })
    }};
}
