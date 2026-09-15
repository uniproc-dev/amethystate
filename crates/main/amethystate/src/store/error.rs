use error_stack::Report;

pub use amethystate_core::failure::{
    IntoStorageReport, Occupied, StorageError, StorageResult, one_line,
};

impl IntoStorageReport for crate::MigrationError {
    fn into_report(self) -> Report<StorageError> {
        Report::new(self).change_context(StorageError::Migrate)
    }
}
