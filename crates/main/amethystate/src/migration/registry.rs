use crate::MigrationContext;
use crate::migration::fields::FieldDescriptor;
use crate::store::StaticPath;
use std::sync::OnceLock;

#[derive(Clone)]
pub struct MigrationStepEntry {
    pub prefix: StaticPath,
    pub target_version: u32,
    pub description: &'static str,
    pub struct_name: &'static str,
    pub fields: &'static [FieldDescriptor],
    pub run: fn(&mut MigrationContext) -> crate::migration::StepResult<()>,
}

inventory::collect!(MigrationStepEntry);

/// Every step this binary carries, the way
/// [`schema::declarations`](crate::schema::declarations) hands over the
/// declarations: one reader of the linker's list, walked once.
///
/// A step written `#[migrate(explicit)]` was never submitted, so it is not in
/// here and reaches a builder through
/// [`add_steps`](crate::migration::builder::MigrationBuilder::add_steps).
pub fn compiled_steps() -> &'static [&'static MigrationStepEntry] {
    static COMPILED: OnceLock<Vec<&'static MigrationStepEntry>> = OnceLock::new();

    COMPILED.get_or_init(|| inventory::iter::<MigrationStepEntry>.into_iter().collect())
}
