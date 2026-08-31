use crate::migration::fields::FieldDescriptor;
use crate::migration::provided::Provided;
use crate::migration::registry::{MigrationDependency, MigrationStepEntry};
use crate::migration::set::MigrationSet;
use crate::store::StorageResult;
use crate::{MigrationContext, MigrationPlan, StateScope};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub struct MigrationBuilder {
    prefixes: HashMap<String, PrefixPlan>,
    provided: Provided,
}

/// Migration plan for a single database prefix.
#[derive(Default)]
pub(crate) struct PrefixPlan {
    migrator: MigrationPlan,
    dependencies: BTreeSet<String>,
    pub(crate) fields: &'static [FieldDescriptor],
    pub(crate) schema_hash: u32,
}

pub struct PrefixMigrationBuilder<'a> {
    builder: &'a mut MigrationBuilder,
    prefix: String,
}

impl MigrationBuilder {
    /// Picks up every step declared with `#[migrate]` anywhere in the binary.
    ///
    /// [`StoreBuilder::build_with_migration`](crate::StoreBuilder::build_with_migration)
    /// calls this; a store opened with plain
    /// [`build`](crate::StoreBuilder::build) runs only the steps added by hand.
    ///
    /// This is the linker's answer to the question:
    /// [`inventory`](https://docs.rs/inventory) collects at link time, and a
    /// step written `#[migrate(explicit)]` stays out of it and is handed over
    /// through [`MigrationBuilder::add_steps`] instead.
    pub fn collect_codegen(&mut self) -> &mut Self {
        use crate::migration::registry::MigrationStepEntry;

        self.add_steps(inventory::iter::<MigrationStepEntry>)
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
        use std::collections::HashSet;
        let mut groups: HashMap<&'static str, Vec<&'a MigrationStepEntry>> = HashMap::new();

        for entry in steps {
            groups.entry(entry.prefix).or_default().push(entry);
        }

        for (prefix, steps) in groups {
            let mut latest_hash = 0;
            let mut max_v = 0;
            let mut latest_fields: &'static [FieldDescriptor] = &[];
            let mut merged_deps = HashSet::new();

            for step in &steps {
                if step.target_version >= max_v {
                    max_v = step.target_version;
                    latest_hash = step.schema_hash;
                    latest_fields = step.fields;
                }

                for dep in step.dependencies {
                    merged_deps.insert(*dep);
                }

                if step.target_version > 0 {
                    self.for_prefix(prefix)
                        .step(step.target_version, step.description, step.run);
                }
            }

            let plan = self.prefix_plan(prefix);
            plan.schema_hash = latest_hash;
            plan.fields = latest_fields;
            for dep in merged_deps {
                plan.dependencies.insert(dep.to_string());
            }
        }
        self
    }

    /// Adds steps for a struct's own prefix, taken from its
    /// [`StateScope`] rather than written out.
    pub fn for_node<T: StateScope>(&mut self) -> PrefixMigrationBuilder<'_> {
        self.for_prefix(T::KEY)
    }

    /// Adds steps for a prefix named directly, rather than taken from a type.
    ///
    /// For a prefix whose struct is not in scope here, and for data no live
    /// struct declares at all - a section being retired still needs its keys
    /// moved or dropped.
    pub fn for_prefix(&mut self, prefix: impl Into<String>) -> PrefixMigrationBuilder<'_> {
        PrefixMigrationBuilder {
            builder: self,
            prefix: prefix.into(),
        }
    }

    pub(crate) fn prefix_plan(&mut self, prefix: &str) -> &mut PrefixPlan {
        self.prefixes.entry(prefix.to_string()).or_default()
    }

    /// Hands a value to every step this builder's migrations produce.
    pub fn provide<T: std::any::Any>(&mut self, value: T) {
        self.provided.insert(value);
    }

    pub(crate) fn into_set(self) -> MigrationSet {
        let mut set = MigrationSet::default();
        let mut prefixes = self.prefixes.into_iter().collect::<Vec<_>>();

        prefixes.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (prefix, plan) in prefixes {
            let deps: Vec<&str> = plan.dependencies.iter().map(|s| s.as_str()).collect();
            set = set.add(prefix, plan.migrator, plan.schema_hash, plan.fields, &deps);
        }

        set.take_provided(self.provided);
        set
    }
}

impl PrefixMigrationBuilder<'_> {
    /// Declares that this prefix's steps must run after another's.
    ///
    /// The engine orders prefixes by these edges, so a step that reads a value
    /// another migration produces is not left racing it. A cycle is refused
    /// rather than resolved: opening the store fails with
    /// [`MigrationError::Cycle`](crate::MigrationError::Cycle), naming one of
    /// the prefixes in it.
    ///
    /// ```rust,ignore
    /// #[amethystate(prefix = "net", version = 2)]
    /// pub struct NetworkState {
    ///     #[amestate(default = 8080)]
    ///     pub port: u16,
    /// }
    ///
    /// #[amethystate(prefix = "ui", version = 2)]
    /// pub struct Dashboard {
    ///     #[amestate(default = 8080)]
    ///     pub proxy_port: u16,
    /// }
    ///
    /// // `net` may itself be moving its port in this same run, so read it
    /// // only after that has happened.
    /// m.for_node::<Dashboard>()
    ///     .depends_on::<NetworkState>()
    ///     .step(2, "adopt the port the network migration settled on", |ctx| {
    ///         let port: u16 = ctx.global_get("net.port")?.unwrap_or(8080);
    ///         ctx.set("proxy_port", &port)
    ///     });
    /// ```
    pub fn depends_on<D: MigrationDependency>(&mut self) -> &mut Self {
        let plan = self.builder.prefix_plan(&self.prefix);
        D::register(&mut plan.dependencies);
        self
    }

    /// [`PrefixMigrationBuilder::depends_on`] against a prefix named as a
    /// string, for one no type describes.
    pub fn depends_on_raw(&mut self, dependency: impl Into<String>) -> &mut Self {
        self.builder
            .prefix_plan(&self.prefix)
            .dependencies
            .insert(dependency.into());
        self
    }

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
    /// let store = StoreBuilder::new(path)
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
    ///     .build_with_migration()?;
    /// ```
    pub fn step<F>(&mut self, target_version: u32, description: &str, run: F) -> &mut Self
    where
        F: Fn(&mut MigrationContext) -> StorageResult<()> + Send + Sync + 'static,
    {
        let plan = self.builder.prefix_plan(&self.prefix);
        let migrator = std::mem::take(&mut plan.migrator);
        plan.migrator = migrator.step(target_version, description, run);
        self
    }
}
