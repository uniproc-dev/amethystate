use crate::AmeBackendSync;
use crate::FieldCore;
use crate::facts::Facts;
use crate::path::StorePath;
use crate::primitives::error::{FieldError, ReactiveFieldResult};
use crate::primitives::field_core::FieldValue;
use uuid::Uuid;

pub fn field_set<B, T>(
    backend: &B,
    core: &FieldCore<T>,
    path: StorePath,
    value: T,
    source: Option<Uuid>,
) -> ReactiveFieldResult<()>
where
    B: AmeBackendSync,
    T: FieldValue,
{
    let change = core
        .run_interceptors(path.clone(), value, source)
        .map_err(|said| FieldError::intercepted(&path, said))?;

    backend
        .set_owned_with_source(path.clone(), &change.new_value, change.source)
        .attach_key(&path)
        .map_err(|why| FieldError::from_store(&path, why))?;

    Ok(())
}

pub fn field_apply_remote_value<T>(core: &FieldCore<T>, value: T, source: Option<Uuid>)
where
    T: Clone + 'static,
{
    core.signal.set_forwarded(value, source);
}
