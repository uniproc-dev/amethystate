use super::MigrationPlan;
use crate::migration::fields::FieldDescriptor;
use crate::migration::provided::Provided;
use crate::store::facts::Facts;
use crate::store::{StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use std::collections::HashMap;

#[derive(Default)]
pub struct MigrationSet {
    migrators: HashMap<String, MigrationPlan>,
    targets: HashMap<String, (u32, &'static [FieldDescriptor])>,

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
    pub fn add(
        mut self,
        prefix: impl Into<String>,
        migrator: MigrationPlan,
        fields: &'static [FieldDescriptor],
    ) -> Self {
        let prefix = prefix.into();

        let target_version = migrator
            .steps
            .iter()
            .map(|s| s.target_version())
            .max()
            .unwrap_or(0);

        self.targets
            .insert(prefix.clone(), (target_version, fields));
        self.migrators.insert(prefix, migrator);

        self
    }

    /// The version and fields the code declares for `prefix`.
    ///
    /// A set that was given steps for the prefix knows this from them. One
    /// that was not - a store opened with
    /// [`build`](crate::StoreBuilder::build), which runs only what was
    /// declared by hand - reads it from the schema instead, because the schema
    /// is what the code says its shape is whether or not anyone collected the
    /// steps to get there.
    ///
    /// A prefix nothing declares answers version zero and no fields, and no
    /// declared places is what stops an undeclared prefix being read as one
    /// that gave all of them up.
    pub(crate) fn get_target(&self, prefix: &str) -> (u32, &'static [FieldDescriptor]) {
        let declared = inventory::iter::<crate::schema::SchemaEntry>
            .into_iter()
            .filter(|entry| entry.prefix.to_string() == prefix);

        let mut furthest = 0;
        let mut fields: &'static [FieldDescriptor] = &[];

        for entry in declared {
            if entry.version >= furthest {
                furthest = entry.version;
                fields = entry.fields;
            }
        }

        match self.targets.get(prefix) {
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

    /// Every prefix this set was given steps for, in a settled order.
    ///
    /// Sorted rather than in the order they arrived, so a run covers the same
    /// prefixes in the same order twice - which matters only for what ends up
    /// grouped with what when a step reaches, and matters there enough that it
    /// should not follow the order a builder happened to be written in.
    pub(crate) fn known_prefixes(&self) -> Vec<String> {
        let mut found: Vec<String> = self.targets.keys().cloned().collect();
        found.sort();
        found
    }

    /// The prefix `full_key` lies under, if this set knows one.
    ///
    /// The longest, because prefixes nest: `app` and `app.ui` can both be
    /// declared, and a key under the second belongs to the second.
    pub(crate) fn owner_of(&self, full_key: &str) -> StorageResult<Option<StorePath>> {
        let key = StorePath::parse_joined(full_key)
            .change_context(StorageError::Path)
            .attach_raw_key(full_key)?;

        let mut owner: Option<StorePath> = None;

        for prefix in self.targets.keys() {
            let Ok(at) = StorePath::parse_joined(prefix) else {
                continue;
            };

            // Longest wins, counted in levels rather than characters: `app.ui`
            // holds more of a key than `app` does, and a name's length says
            // nothing about how far down it reaches.
            if key.starts_with(&at) && owner.as_ref().is_none_or(|held| held.len() < at.len()) {
                owner = Some(at);
            }
        }

        Ok(owner)
    }

    pub(crate) fn get_migration_plan(&self, prefix: &str) -> Option<&MigrationPlan> {
        self.migrators.get(prefix)
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

    #[test]
    fn the_prefixes_come_back_sorted_whatever_order_they_were_added_in() {
        let one = MigrationSet::default()
            .add("x", dummy_migrator(), EMPTY_FIELDS)
            .add("a", dummy_migrator(), EMPTY_FIELDS);

        let other = MigrationSet::default()
            .add("a", dummy_migrator(), EMPTY_FIELDS)
            .add("x", dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(one.known_prefixes(), vec!["a", "x"]);
        assert_eq!(one.known_prefixes(), other.known_prefixes());
    }

    #[test]
    fn a_key_belongs_to_the_longest_prefix_that_starts_it() {
        let set = MigrationSet::default()
            .add("app", dummy_migrator(), EMPTY_FIELDS)
            .add("app.ui", dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(
            set.owner_of("app.ui.theme").unwrap(),
            Some(StorePath::from_segments(["app", "ui"]))
        );
        assert_eq!(
            set.owner_of("app.net").unwrap(),
            Some(StorePath::segment("app"))
        );
    }

    #[test]
    fn a_key_under_nothing_declared_belongs_to_nobody() {
        let set = MigrationSet::default().add("app", dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(set.owner_of("other.thing").unwrap(), None);
    }

    #[test]
    fn a_prefix_is_not_the_owner_of_a_name_it_merely_starts() {
        let set = MigrationSet::default().add("app", dummy_migrator(), EMPTY_FIELDS);

        assert_eq!(
            set.owner_of("application.thing").unwrap(),
            None,
            "`app` starts the string `application` and starts none of its levels"
        );
    }

    #[test]
    fn test_target_info_retrieval() {
        static TEST_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf(&["id"], "id", "u64")];

        let migrator = MigrationPlan::new().step(1, "init", |_| Ok(()));
        let set = MigrationSet::default().add("app", migrator, TEST_FIELDS);

        let (v, f) = set.get_target("app");
        assert_eq!(v, 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name.as_str(), "id");
        assert_eq!(f[0].type_name, "u64");
    }
}
