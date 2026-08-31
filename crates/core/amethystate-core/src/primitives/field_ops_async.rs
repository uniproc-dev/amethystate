use crate::facts::Facts;
use crate::path::StorePath;
use crate::primitives::error::{FieldError, ReactiveFieldResult};
use crate::primitives::field_core::FieldValue;
use crate::{AmeBackendAsync, FieldCore};
use error_stack::ResultExt;
use uuid::Uuid;

pub async fn field_set_async<B, T>(
    backend: &B,
    core: &FieldCore<T>,
    path: StorePath,
    value: T,
    source: Option<Uuid>,
) -> ReactiveFieldResult<()>
where
    B: AmeBackendAsync,
    T: FieldValue,
{
    let change = core
        .run_interceptors(path.clone(), value, source)
        .map_err(FieldError::intercepted)
        .attach_key(&path)?;

    backend
        .set_owned_with_source(path.clone(), &change.new_value, change.source)
        .await
        .change_context(FieldError::Storage)
        .attach_key(&path)?;

    Ok(())
}
