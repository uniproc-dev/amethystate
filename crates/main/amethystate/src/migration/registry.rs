use crate::MigrationContext;
use crate::migration::fields::FieldDescriptor;
use crate::store::StateScope;
use crate::store::StorageResult;
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct MigrationStepEntry {
    pub prefix: &'static str,
    pub target_version: u32,
    pub description: &'static str,
    pub dependencies: &'static [&'static str],
    pub struct_name: &'static str,
    pub fields: &'static [FieldDescriptor],
    pub schema_hash: u32,
    pub run: fn(&mut MigrationContext) -> StorageResult<()>,
}

inventory::collect!(MigrationStepEntry);

pub trait MigrationDependency {
    fn register(deps: &mut BTreeSet<String>);
}

impl<T: StateScope> MigrationDependency for T {
    fn register(deps: &mut BTreeSet<String>) {
        deps.insert(T::KEY.to_string());
    }
}

impl MigrationDependency for () {
    fn register(_deps: &mut BTreeSet<String>) {}
}

macro_rules! impl_migration_dependency_tuple {
    ($($ty:ident),*) => {
        impl<$($ty: MigrationDependency),*> MigrationDependency for ($($ty,)*) {
            fn register(deps: &mut BTreeSet<String>) {
                $($ty::register(deps);)*
            }
        }
    };
}

impl_migration_dependency_tuple!(A);
impl_migration_dependency_tuple!(A, B);
impl_migration_dependency_tuple!(A, B, C);
impl_migration_dependency_tuple!(A, B, C, D);
impl_migration_dependency_tuple!(A, B, C, D, E);
