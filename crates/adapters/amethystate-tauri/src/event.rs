use crate::TauriResult;
use futures::{Stream, StreamExt, channel::mpsc};
use serde::de::DeserializeOwned;
use wasm_bindgen::{JsValue, prelude::*};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event<T> {
    pub payload: T,
}

#[allow(unused)]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", content = "label")]
pub enum EventTarget {
    Any,
    AnyLabel(String),
    App,
    Window(String),
    Webview(String),
    WebviewWindow(String),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct Options {
    pub target: EventTarget,
}

mod inner {
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(module = "/src/js/event.js")]
    extern "C" {
        #[wasm_bindgen(catch)]
        pub async fn listen(
            event: &str,
            handler: &Closure<dyn FnMut(JsValue)>,
            options: JsValue,
        ) -> Result<JsValue, JsValue>;
    }
}

pub struct Listen<T> {
    rx: mpsc::UnboundedReceiver<Event<T>>,
    unlisten: js_sys::Function,
    _callback_keep_alive: Closure<dyn FnMut(JsValue)>,
}

impl<T> Drop for Listen<T> {
    fn drop(&mut self) {
        self.unlisten.call0(&JsValue::NULL).ok();
    }
}

impl<T> Stream for Listen<T> {
    type Item = Event<T>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_next_unpin(cx)
    }
}

#[inline(always)]
pub async fn listen<T>(event: &str) -> TauriResult<impl Stream<Item = Event<T>>>
where
    T: DeserializeOwned + 'static,
{
    let (tx, rx) = mpsc::unbounded::<Event<T>>();

    let closure = Closure::<dyn FnMut(JsValue)>::new(move |raw| {
        let _ = tx.unbounded_send(serde_wasm_bindgen::from_value(raw).unwrap());
    });
    let unlisten = inner::listen(
        event,
        &closure,
        serde_wasm_bindgen::to_value(&Options {
            target: EventTarget::Any,
        })?,
    )
    .await?;

    Ok(Listen {
        rx,
        unlisten: js_sys::Function::from(unlisten),
        _callback_keep_alive: closure,
    })
}
