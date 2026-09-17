use crate::migration::fields::FieldDescriptor;
use crate::migration::provided::Provided;
use crate::migration::registry::MigrationStepEntry;
use crate::migration::set::MigrationSet;
use crate::schema::Lineage;
use crate::{MigrationContext, MigrationPlan, StateScope};
use amethystate_core::path::{StorePath, StorePathError};
use std::collections::HashMap;

#[derive(Default)]
pub struct MigrationBuilder {
    prefixes: HashMap<Lineage, PrefixPlan>,

    /// The `#[migrate]` steps already taken, so one handed over twice - by
    /// [`MigrationBuilder::collect_codegen`] in the application and again by
    /// `migrate` - stays one step rather than becoming two.
    compiled: Vec<(Lineage, u32, CompiledStep)>,
    provided: Provided,

    /// The first prefix that would not read as a path, kept until
    /// [`MigrationBuilder::into_set`] can hand it back.
    ///
    /// [`MigrationBuilder::for_prefix`] takes a spelling and returns a builder
    /// to go on writing steps into, so there is nowhere to put a failure at the
    /// call that makes it.
    refused: Option<(String, StorePathError)>,
}

type CompiledStep = fn(&mut MigrationContext) -> crate::migration::StepResult<()>;

#[derive(Default)]
pub(crate) struct PrefixPlan {
    migrator: MigrationPlan,
    pub(crate) fields: &'static [FieldDescriptor],
}

pub struct PrefixMigrationBuilder<'a> {
    builder: &'a mut MigrationBuilder,
    lineage: Lineage,
}

impl MigrationBuilder {
    /// Picks up every step declared with `#[migrate]` anywhere in the binary.
    ///
    /// [`StoreBuilder::migrate`](crate::StoreBuilder::migrate) calls this; a
    /// store opened with [`build`](crate::StoreBuilder::build) runs no step at
    /// all.
    ///
    /// This is the linker's answer to the question:
    /// [`inventory`](https://docs.rs/inventory) collects at link time, and a
    /// step written `#[migrate(explicit)]` stays out of it and is handed over
    /// through [`MigrationBuilder::add_steps`] instead.
    pub fn collect_codegen(&mut self) -> &mut Self {
        self.add_steps(crate::migration::registry::compiled_steps().iter().copied())
    }

    /// Takes steps as given, for an application that would rather name them
    /// than have them found.
    ///
    /// A step written `#[migrate(explicit)]` is not submitted to `inventory`
    /// and so is invisible to [`MigrationBuilder::collect_codegen`]; the macro
    /// leaves a `const` named for the function instead, and this is where it
    /// goes.
    ///
    /// ```ignore
    /// StoreBuilder::new("./app")
    ///     .migrations(|m| { m.add_steps(&[SETTINGS_TO_V2, PANELS_TO_V3]); })
    ///     .build()?;
    /// ```
    pub fn add_steps<'a>(
        &mut self,
        steps: impl IntoIterator<Item = &'a MigrationStepEntry>,
    ) -> &mut Self {
        let mut groups: HashMap<Lineage, Vec<&'a MigrationStepEntry>> = HashMap::new();

        for entry in steps {
            let lineage = Lineage {
                prefix: entry.prefix.path(),
                id: entry.id.map(str::to_string),
            };
            groups.entry(lineage).or_default().push(entry);
        }

        for (lineage, steps) in groups {
            let mut max_v = 0;
            let mut latest_fields: &'static [FieldDescriptor] = &[];

            for step in &steps {
                if step.target_version >= max_v {
                    max_v = step.target_version;
                    latest_fields = step.fields;
                }

                if step.target_version > 0 && !self.already_took(&lineage, step) {
                    self.compiled
                        .push((lineage.clone(), step.target_version, step.run));
                    self.for_lineage(lineage.clone()).step(
                        step.target_version,
                        step.description,
                        step.run,
                    );
                }
            }

            self.prefix_plan(&lineage).fields = latest_fields;
        }
        self
    }

    fn already_took(&self, lineage: &Lineage, step: &MigrationStepEntry) -> bool {
        self.compiled.iter().any(|(at, version, run)| {
            at == lineage && *version == step.target_version && std::ptr::fn_addr_eq(*run, step.run)
        })
    }

    /// Adds steps for a struct's own line of declarations, taken from its
    /// [`StateScope`] - its prefix and its `id` - rather than written out.
    pub fn for_node<T: StateScope>(&mut self) -> PrefixMigrationBuilder<'_> {
        self.for_lineage(Lineage {
            prefix: T::PATH,
            id: T::ID.map(str::to_string),
        })
    }

    pub(crate) fn for_lineage(&mut self, lineage: Lineage) -> PrefixMigrationBuilder<'_> {
        PrefixMigrationBuilder {
            builder: self,
            lineage,
        }
    }

    /// Adds steps for a prefix named directly, rather than taken from a type.
    ///
    /// For a prefix whose struct is not in scope here, and for data no live
    /// struct declares at all - a section being retired still needs its keys
    /// moved or dropped.
    ///
    /// The spelling means what it means in a declaration: `app.ui` is two
    /// levels, and a level holding a separator is written with an escape
    /// before it. A spelling that is no path is kept and handed back where the
    /// builder is turned into a set, so the store refuses to open rather than
    /// running a set with a prefix nothing can address.
    pub fn for_prefix(&mut self, prefix: impl AsRef<str>) -> PrefixMigrationBuilder<'_> {
        let prefix = self.parsed(prefix.as_ref());
        self.for_lineage(Lineage::unnamed(prefix))
    }

    /// The same for the line declared with `id` at a prefix more than one
    /// struct shares.
    pub fn for_named(
        &mut self,
        prefix: impl AsRef<str>,
        id: impl Into<String>,
    ) -> PrefixMigrationBuilder<'_> {
        let prefix = self.parsed(prefix.as_ref());
        self.for_lineage(Lineage::named(prefix, id))
    }

    fn parsed(&mut self, written: &str) -> StorePath {
        match StorePath::parse_joined(written) {
            Ok(at) => at,
            Err(why) => {
                self.refused
                    .get_or_insert_with(|| (written.to_string(), why));
                StorePath::root()
            }
        }
    }

    pub(crate) fn prefix_plan(&mut self, lineage: &Lineage) -> &mut PrefixPlan {
        self.prefixes.entry(lineage.clone()).or_default()
    }

    /// Hands a value to every step this builder's migrations produce.
    pub fn provide<T: std::any::Any>(&mut self, value: T) {
        self.provided.insert(value);
    }

    pub(crate) fn into_set(self) -> Result<MigrationSet, (String, StorePathError)> {
        if let Some(refused) = self.refused {
            return Err(refused);
        }

        let mut set = MigrationSet::default();
        let mut prefixes = self.prefixes.into_iter().collect::<Vec<_>>();

        prefixes.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (lineage, plan) in prefixes {
            set = set.add(lineage, plan.migrator, plan.fields);
        }

        set.take_provided(self.provided);
        Ok(set)
    }
}

impl PrefixMigrationBuilder<'_> {
    /// Adds one step, taking the data to `target_version`.
    ///
    /// Steps run in ascending version order, and only those above the version
    /// already recorded for the prefix, so a store several versions behind
    /// walks through each in turn. The description is what shows up in
    /// [`MigrationReport`](crate::MigrationReport) and in the log.
    ///
    /// ```rust,ignore
    /// #[amethystate(prefix = "profile", version = 3)]
    /// pub struct Profile {
    ///     #[amestate(default = String::new())]
    ///     pub display_name: String,
    ///
    ///     // Added in version 3, derived from the name already stored.
    ///     #[amestate(default = String::new())]
    ///     pub initials: String,
    /// }
    ///
    /// let (store, report) = StoreBuilder::new(path)
    ///     .migrations(|m| {
    ///         m.for_node::<Profile>()
    ///             .step(3, "derive initials", |ctx| {
    ///                 let display_name = ctx.get::<String>("display_name")?.unwrap_or_default();
    ///                 let initials: String = display_name
    ///                     .split_whitespace()
    ///                     .filter_map(|part| part.chars().next())
    ///                     .collect();
    ///                 ctx.set("initials", &initials)
    ///             });
    ///     })
    ///     .migrate()?;
    /// ```
    pub fn step<F>(&mut self, target_version: u32, description: &str, run: F) -> &mut Self
    where
        F: Fn(&mut MigrationContext) -> crate::migration::StepResult<()> + Send + Sync + 'static,
    {
        let plan = self.builder.prefix_plan(&self.lineage);
        let migrator = std::mem::take(&mut plan.migrator);
        plan.migrator = migrator.step(target_version, description, run);
        self
    }
}
