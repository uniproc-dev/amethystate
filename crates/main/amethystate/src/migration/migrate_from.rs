use crate::MigrationContext;

pub trait MigrateFrom<TOld>: Sized {
    const RENAMES: &'static [(&'static str, &'static str)] = &[];

    fn migrate(old: TOld, ctx: &mut MigrationContext) -> crate::migration::StepResult<Self>;
}
