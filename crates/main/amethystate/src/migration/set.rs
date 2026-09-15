use super::MigrationPlan;
use crate::migration::fields::FieldDescriptor;
use crate::migration::provided::Provided;
use crate::schema::Lineage;
use amethystate_core::path::StorePath;
use std::collections::HashMap;

#[derive(Default)]
pub struct MigrationSet {
    migrators: HashMap<Lineage, MigrationPlan>,
    targets: HashMap<Lineage, (u32, &'static [FieldDescriptor])>,

    /// What the steps need from outside the store. Carried here because a
    /// step is a bare `fn` with nothing to capture, and because these exist
    /// for the migrations and nothing else.
    provided: Provided,
}

impl MigrationSet {
    /// Hands a value to every step this set runs. See
    /// [`StoreBuilder::provide`](crate::StoreBuilder::provide).
    pub fn provide<T: std::any::Any>(&mut self, value: T) {
        self.provided.insert(value);
    }

    pub(crate) fn take_provided(&mut self, provided: Provided) {
        self.provided = provided;
    }

    pub(crate) fn provided(&self) -> &Provided {
        &self.provided
    }

    /// Steps for one line of declarations - a prefix alone for its unnamed
    /// line, or a [`Lineage`] for one declared with an `id`.
    pub fn add(
        mut self,
        lineage: impl Into<Lineage>,
        migrator: MigrationPlan,
        fields: &'static [FieldDescriptor],
    ) -> Self {
        let lineage = lineage.into();

        let target_version = migrator
            .steps
            .iter()
            .map(|s| s.target_version())
            .max()
            .unwrap_or(0);

        self.targets
            .insert(lineage.clone(), (target_version, fields));
        self.migrators.insert(lineage, migrator);

        self
    }

    /// The version and fields the code declares for `lineage`.
    ///
    /// A set that was given steps for the line knows this from them. One that
    /// was not - a store opened with [`build`](crate::StoreBuilder::build),
    /// which runs only what was declared by hand - reads it from the schema
    /// instead, because the schema is what the code says its shape is whether
    /// or not anyone collected the steps to get there.
    ///
    /// A line nothing declares answers version zero and no fields, and no
    /// declared places is what stops an undeclared prefix being read as one
    /// that gave all of them up.
    pub(crate) fn get_target(&self, lineage: &Lineage) -> (u32, &'static [FieldDescriptor]) {
        let mut furthest = 0;
        let mut fields: &'static [FieldDescriptor] = &[];

        for entry in crate::schema::declarations_of(lineage) {
            if entry.version >= furthest {
                furthest = entry.version;
                fields = entry.fields;
            }
        }

        match self.targets.get(lineage) {
            Some((planned, of_the_plan)) => {
                let fields = match fields.is_empty() {
                    true => *of_the_plan,
                    false => fields,
                };
                (furthest.max(*planned), fields)
            }
            None => (furthest, fields),
        }
    }

    /// Every line this set was given steps for, in a settled order.
    ///
    /// Only those. A pass migrates what it was asked to migrate, and stamping a
    /// version on a line nobody handed it steps for would record that the data
    /// is at the declared version when nothing brought it there - after which
    /// the run that does have the steps sees nothing to do. Drift at a line
    /// with no steps is looked at separately, by
    /// [`MigrationEngine::drift_where_no_step_runs`](crate::migration::engine::MigrationEngine::drift_where_no_step_runs).
    ///
    /// Sorted rather than in the order they arrived, so a run covers the same
    /// lines in the same order twice - which matters only for what ends up
    /// grouped with what when a step reaches, and matters there enough that it
    /// should not follow the order a builder happened to be written in.
    pub(crate) fn known_lineages(&self) -> Vec<Lineage> {
        let mut found: Vec<Lineage> = self.targets.keys().cloned().collect();
        found.sort();
        found
    }

    /// The lines this set was given steps for at `prefix`, in the same order.
    pub(crate) fn lineages_at(&self, prefix: &StorePath) -> Vec<Lineage> {
        let mut found: Vec<Lineage> = self
            .targets
            .keys()
            .filter(|lineage| lineage.prefix == *prefix)
            .cloned()
            .collect();
        found.sort();
        found
    }

    /// The prefix `full_key` lies under, if this set knows one.
    ///
    /// The longest, because prefixes nest: `app` and `app.ui` can both be
    /// declared, and a key under the second belongs to the second.
    pub(crate) fn owner_of(&self, key: &StorePath) -> Option<StorePath> {
        let mut owner: Option<&StorePath> = None;

        for at in self.targets.keys().map(|lineage| &lineage.prefix) {
            // Longest wins, counted in levels rather than characters: `app.ui`
            // holds more of a key than `app` does, and a name's length says
            // nothing about how far down it reaches.
            if key.starts_with(at) && owner.is_none_or(|held| held.len() < at.len()) {
                owner = Some(at);
            }
        }

        owner.cloned()
    }

    pub(crate) fn get_migration_plan(&self, lineage: &Lineage) -> Option<&MigrationPlan> {
        self.migrators.get(lineage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::migration::fields::FieldDescriptor;

    const EMPTY_FIELDS: &[FieldDescriptor] = &[];

    fn dummy_migrator() -> MigrationPlan {
        MigrationPlan::new()
    }

    fn at(joined: &str) -> StorePath {
        StorePath::parse_joined(joined).expect("a path the tests wrote themselves")
    }

    #[test]
    fn the_lines_come_back_sorted_whatever_order_they_were_added_in() {
        let one = MigrationSet::default()
            .add(at("x"), dummy_migrator(), EMPTY_FIELDS)
            .add(Lineage::named(at("a"), "b"), dummy_migrator(), EMPTY_FIELDS)
            .add(at("a"), dummy_migrator(), EMPTY_FIELDS);

        let other = MigrationSet::default()
            .add(at("a"), dummy_migrator(), EMPTY_FIELDS)
            .add(at("x"), dummy_migrator(), EMPTY_FIELDS)
            .add(Lineage::named(at("a"), "b"), dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(
            one.known_lineages(),
            vec![
                Lineage::unnamed(at("a")),
                Lineage::named(at("a"), "b"),
                Lineage::unnamed(at("x")),
            ]
        );
        assert_eq!(one.known_lineages(), other.known_lineages());
    }

    #[test]
    fn two_lines_at_one_prefix_keep_their_own_steps() {
        let set = MigrationSet::default()
            .add(
                Lineage::named(at("app"), "left"),
                MigrationPlan::new().step(2, "left", |_| Ok(())),
                EMPTY_FIELDS,
            )
            .add(
                Lineage::named(at("app"), "right"),
                MigrationPlan::new().step(5, "right", |_| Ok(())),
                EMPTY_FIELDS,
            );

        assert_eq!(set.get_target(&Lineage::named(at("app"), "left")).0, 2);
        assert_eq!(set.get_target(&Lineage::named(at("app"), "right")).0, 5);
        assert_eq!(
            set.lineages_at(&at("app")),
            vec![
                Lineage::named(at("app"), "left"),
                Lineage::named(at("app"), "right"),
            ]
        );
    }

    #[test]
    fn a_key_belongs_to_the_longest_prefix_that_starts_it() {
        let set = MigrationSet::default()
            .add(at("app"), dummy_migrator(), EMPTY_FIELDS)
            .add(at("app.ui"), dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(
            set.owner_of(&at("app.ui.theme")),
            Some(StorePath::from_segments(["app", "ui"]))
        );
        assert_eq!(
            set.owner_of(&at("app.net")),
            Some(StorePath::segment("app"))
        );
    }

    #[test]
    fn a_key_under_nothing_declared_belongs_to_nobody() {
        let set = MigrationSet::default().add(at("app"), dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(set.owner_of(&at("other.thing")), None);
    }

    #[test]
    fn a_prefix_is_not_the_owner_of_a_name_it_merely_starts() {
        let set = MigrationSet::default().add(at("app"), dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(
            set.owner_of(&at("application.thing")),
            None,
            "`app` starts the string `application` and starts none of its levels"
        );
    }

    #[test]
    fn a_name_holding_a_separator_is_one_level_and_owns_nothing_under_the_two() {
        let set = MigrationSet::default().add(at("app"), dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(
            set.owner_of(&StorePath::segment("app.ui")),
            None,
            "one level called `app.ui` is not a key under `app`"
        );
    }

    #[test]
    fn test_target_info_retrieval() {
        static TEST_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf(&["id"], "id", "u64")];

        let migrator = MigrationPlan::new().step(1, "init", |_| Ok(()));
        let set = MigrationSet::default().add(at("app"), migrator, TEST_FIELDS);

        let (v, f) = set.get_target(&at("app").into());
        assert_eq!(v, 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name.as_str(), "id");
        assert_eq!(f[0].type_name, "u64");
    }
}
