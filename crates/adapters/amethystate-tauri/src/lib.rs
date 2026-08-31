use crate::event::Event;
use amethystate_core::path::StorePath;
use amethystate_core::primitives::map_core::{ReactiveMapKey, ReactiveMapValue};
use amethystate_core::{AmeBackendAsync, AsyncSubscriptionBackend, SubscriptionHandle};
use amethystate_core::{FieldCore, MapChange, ReactiveMapCore};
use error_stack::Report;
use futures::StreamExt;
use futures::future::AbortHandle;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use uuid::Uuid;
pub(crate) type TauriResult<T> = std::result::Result<T, Error>;

mod core;
mod error;
mod event;
pub use error::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TauriBackend;

impl TauriBackend {
    pub fn new() -> Self {
        Self
    }
}

/// The plugin answers with a string, and the trait wants an error a report can
/// carry. This is the one place the two meet.
fn commanded(message: String, command: &str, path: &StorePath) -> Report<Error> {
    Report::new(Error::Command(message))
        .attach(format!("command: {command}"))
        .attach(format!("path: {path}"))
}

impl AmeBackendAsync for TauriBackend {
    type Error = Error;
    type Raw = serde_json::Value;

    async fn get<T>(&self, path: &StorePath) -> Result<Option<T>, Report<Self::Error>>
    where
        T: DeserializeOwned,
    {
        #[derive(Serialize)]
        struct GetArgs<'a> {
            key: &'a str,
        }

        const COMMAND: &str = "plugin:amethystate|amethystate_get";

        let raw = core::invoke_result::<Option<serde_json::Value>, String>(COMMAND, &GetArgs {
            key: path.as_str(),
        })
        .await
        .map_err(|e| commanded(e, COMMAND, path))?;

        raw.map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                Report::new(Error::Serde(e.to_string())).attach(format!("path: {path}"))
            })
    }

    async fn set<T>(&self, path: &StorePath, value: &T) -> Result<(), Report<Self::Error>>
    where
        T: Serialize,
    {
        self.set_with_source(path, value, None).await
    }

    async fn set_with_source<T: Serialize>(
        &self,
        path: &StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        #[derive(Serialize)]
        struct SetArgs<'a> {
            key: &'a str,
            value: serde_json::Value,
            source: Option<Uuid>,
        }

        const COMMAND: &str = "plugin:amethystate|amethystate_set";

        let value = serde_json::to_value(value).map_err(|e| {
            Report::new(Error::Serde(e.to_string())).attach(format!("path: {path}"))
        })?;
        core::invoke_result::<(), String>(COMMAND, &SetArgs {
            key: path.as_str(),
            value,
            source,
        })
        .await
        .map_err(|e| commanded(e, COMMAND, path))
    }

    async fn set_owned_with_source<T: Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        self.set_with_source(&path, value, source).await
    }

    async fn delete(&self, path: &StorePath) -> Result<(), Report<Self::Error>> {
        self.delete_with_source(path, None).await
    }

    async fn delete_with_source(
        &self,
        path: &StorePath,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        #[derive(Serialize)]
        struct DeleteArgs<'a> {
            key: &'a str,
            source: Option<Uuid>,
        }

        const COMMAND: &str = "plugin:amethystate|amethystate_delete";

        core::invoke_result::<(), String>(COMMAND, &DeleteArgs {
            key: path.as_str(),
            source,
        })
        .await
        .map_err(|e| commanded(e, COMMAND, path))
    }

    async fn delete_prefix(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        #[derive(Serialize)]
        struct DeletePrefixArgs<'a> {
            prefix: &'a str,
            source: Option<Uuid>,
        }

        const COMMAND: &str = "plugin:amethystate|amethystate_delete_prefix";

        core::invoke_result::<(), String>(COMMAND, &DeletePrefixArgs {
            prefix: prefix.as_str(),
            source,
        })
        .await
        .map_err(|e| commanded(e, COMMAND, prefix))
    }

    async fn scan_keys(
        &self,
        prefix: &StorePath,
    ) -> Result<Vec<StorePath>, Report<Self::Error>> {
        #[derive(Serialize)]
        struct PrefixArgs<'a> {
            prefix: &'a str,
        }

        const COMMAND: &str = "plugin:amethystate|amethystate_scan_keys";

        let keys: Vec<String> = core::invoke_result::<_, String>(COMMAND, &PrefixArgs {
            prefix: prefix.as_str(),
        })
        .await
        .map_err(|e| commanded(e, COMMAND, prefix))?;

        keys.into_iter()
            .map(|key| {
                StorePath::parse_joined(&key).map_err(|e| {
                    Report::new(Error::Serde(e.to_string()))
                        .attach(format!("prefix: {prefix}"))
                        .attach(format!("stored key: {key}"))
                })
            })
            .collect()
    }

    async fn scan_prefix(
        &self,
        prefix: &StorePath,
    ) -> Result<Vec<(StorePath, Self::Raw)>, Report<Self::Error>> {
        #[derive(Serialize)]
        struct PrefixArgs<'a> {
            prefix: &'a str,
        }

        const COMMAND: &str = "plugin:amethystate|amethystate_get_prefix";

        let raw: std::collections::HashMap<String, serde_json::Value> =
            core::invoke_result::<_, String>(COMMAND, &PrefixArgs {
                prefix: prefix.as_str(),
            })
            .await
            .map_err(|e| commanded(e, COMMAND, prefix))?;

        raw.into_iter()
            .map(|(key, value)| {
                let path = StorePath::parse_joined(&key).map_err(|e| {
                    Report::new(Error::Serde(e.to_string()))
                        .attach(format!("prefix: {prefix}"))
                        .attach(format!("stored key: {key}"))
                })?;
                Ok((path, value))
            })
            .collect()
    }

    fn decode<T>(&self, raw: &Self::Raw) -> Result<T, Report<Self::Error>>
    where
        T: DeserializeOwned + Default,
    {
        serde_json::from_value(raw.clone())
            .map_err(|e| Report::new(Error::Serde(e.to_string())))
    }
}

impl AsyncSubscriptionBackend for TauriBackend {
    fn subscribe_field<T>(&self, path: StorePath, core: FieldCore<T>) -> SubscriptionHandle
    where
        T: DeserializeOwned + Clone + Send + Sync + 'static,
    {
        let event_channel = format!("amethystate://{}", path.as_str().replace('.', ":"));
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        wasm_bindgen_futures::spawn_local(async move {
            #[derive(Serialize)]
            struct SubArgs<'a> {
                key: &'a str,
            }

            let _ = core::invoke_result::<(), String>(
                "plugin:amethystate|amethystate_subscribe",
                &SubArgs { key: path.as_str() },
            )
            .await;

            if let Ok(stream) = event::listen::<T>(&event_channel).await {
                let mut aborted_stream =
                    futures::stream::Abortable::new(stream, abort_registration);
                while let Some(Event { payload, .. }) = aborted_stream.next().await {
                    amethystate_core::field_apply_remote_value(&core, payload, None);
                }
            }
        });

        SubscriptionHandle::new(move || abort_handle.abort())
    }

    fn subscribe_map<K, V>(
        &self,
        path: StorePath,
        core: ReactiveMapCore<K, V>,
    ) -> SubscriptionHandle
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        let event_channel = format!("amethystate://{}", path.as_str().replace('.', ":"));
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        wasm_bindgen_futures::spawn_local(async move {
            #[derive(Serialize)]
            struct SubArgs<'a> {
                key: &'a str,
            }

            let _ = core::invoke_result::<(), String>(
                "plugin:amethystate|amethystate_subscribe",
                &SubArgs { key: path.as_str() },
            )
            .await;

            if let Ok(stream) = event::listen::<serde_json::Value>(&event_channel).await {
                let mut aborted_stream =
                    futures::stream::Abortable::new(stream, abort_registration);
                while let Some(Event { payload, .. }) = aborted_stream.next().await {
                    if let Ok(change) = serde_json::from_value::<MapChangeHelper<K, V>>(payload) {
                        let core_change = change.into_core();
                        amethystate_core::map_apply_remote_change(&core, &core_change);
                        core.notify(&core_change);
                    }
                }
            }
        });

        SubscriptionHandle::new(move || abort_handle.abort())
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum MapChangeHelper<K, V> {
    Insert {
        key: K,
        value: V,
        source: Option<Uuid>,
    },
    Update {
        key: K,
        #[serde(rename = "oldValue")]
        old_value: V,
        #[serde(rename = "newValue")]
        new_value: V,
        source: Option<Uuid>,
    },
    Remove {
        key: K,
        #[serde(rename = "oldValue")]
        old_value: V,
        source: Option<Uuid>,
    },
    Clear {
        source: Option<Uuid>,
    },
}

impl<K, V> MapChangeHelper<K, V> {
    fn into_core(self) -> MapChange<K, V> {
        match self {
            MapChangeHelper::Insert { key, value, source } => {
                MapChange::Insert { key, value, source }
            }
            MapChangeHelper::Update {
                key,
                old_value,
                new_value,
                source,
            } => MapChange::Update {
                key,
                old_value,
                new_value,
                source,
            },
            MapChangeHelper::Remove {
                key,
                old_value,
                source,
            } => MapChange::Remove {
                key,
                old_value,
                source,
            },
            MapChangeHelper::Clear { source } => MapChange::Clear { source },
        }
    }
}
