use crate::store::error::StorageError;
use amethystate_core::path::StorePath;
use error_stack::Report;
use parking_lot::RwLock;
use std::fmt;

/// A stored path and the schema that took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub path: StorePath,
    pub by: &'static str,
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} claims {}", self.by, self.path)
    }
}

/// The places that are spoken for, and by whom, so that two owners cannot
/// write over each other.
///
/// A place is taken where a path is composed - the constructor of a field or a
/// map - and belongs to the *name* that took it. So the same schema taking the
/// same place again is a no-op, and reconstructing a struct works.
///
/// Nothing gives a place back. There is no release and no removal: the set only
/// grows, and it grows until the last handle on the store it belongs to is
/// dropped. Two stores in one process hold two of these and refuse each other
/// nothing - which is why this is not the whole of the guard, and
/// [`Kv::guard`](crate::store::Kv) asks the declarations as well.
#[derive(Default)]
pub struct Places {
    /// Sorted by path, always.
    ///
    /// Nothing sorts this: it starts empty and the one insertion goes to the
    /// position [`Vec::partition_point`] names, which is the sorted one. Every
    /// lookup below assumes it - two binary searches and a run - so an
    /// insertion anywhere else would not be slower, it would be wrong.
    taken: RwLock<Vec<Owner>>,
}

/// Two owners over one place, with both sides named.
///
/// Which side is which is the whole of the diagnosis, and a report carrying
/// two [`Owner`] facts leaves the reader to tell them apart by the order they
/// were attached in. Here they are separate fields.
///
/// `held_at` need not be `at`: a place is held by one sitting above it or
/// inside it, and saying which is what makes the collision findable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub at: StorePath,
    pub wanted_by: &'static str,
    pub held_at: StorePath,
    pub held_by: &'static str,
}

impl fmt::Display for Taken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            at,
            wanted_by,
            held_at,
            held_by,
        } = self;

        match at == held_at {
            true => write!(f, "{wanted_by} wants {at}, which {held_by} already holds"),
            false => write!(
                f,
                "{wanted_by} wants {at}, which {held_by} already holds through {held_at}"
            ),
        }
    }
}

impl std::error::Error for Taken {}

/// The refusal as a report, for the error sets that carry one.
pub(crate) fn refused(taken: &Taken) -> Report<StorageError> {
    Report::new(StorageError::Taken)
        .attach(Owner {
            path: taken.held_at.clone(),
            by: taken.held_by,
        })
        .attach(Owner {
            path: taken.at.clone(),
            by: taken.wanted_by,
        })
}

impl Places {
    /// Records that `by` owns `path` and everything under it, or refuses.
    ///
    /// Two places meet when one holds the other, so a place already taken can
    /// meet this one in exactly two ways, and the search is those two:
    ///
    /// ```text
    ///   taken, sorted             taking `pot.ato`
    ///   ------------------------------------------------------------
    ///   lid                       before it, and cannot hold it
    ///   pot                       ABOVE - looked up, by name
    ///   pot!luck                  sorts between, holds nothing here
    ///   pot.ato                   the place itself
    ///   pot.ato.skin              BELOW - found by walking the run
    ///   potato                    past the run, and cannot be under it
    /// ```
    ///
    /// Upwards is a lookup per ancestor and downwards a walk. Refuses with both
    /// sides named rather than with a report to be taken apart, boxed because
    /// four paths and names would widen every `Result` on the way out.
    pub fn take(&self, path: &StorePath, by: &'static str) -> Result<(), Box<Taken>> {
        let refuse = |other: &Owner| {
            Box::new(Taken {
                at: path.clone(),
                wanted_by: by,
                held_at: other.path.clone(),
                held_by: other.by,
            })
        };

        let mut taken = self.taken.write();

        // A place that holds `path` sits at `path` itself or at one of its
        // ancestors, since containment is a prefix at a level boundary. They
        // are looked up rather than scanned back to, because the run between an
        // ancestor and `path` is not all ancestors: a sibling subtree of a
        // shallower level sits in it, so a walk backwards that stopped at the
        // first non-prefix would stop before it reached the ancestor.
        for one in path.upwards() {
            if let Ok(found) = taken.binary_search_by(|c| c.path.cmp(&one)) {
                let other = &taken[found];
                if other.by != by {
                    return Err(refuse(other));
                }
                if other.path == *path {
                    return Ok(());
                }
            }
        }

        // Downwards it is one run and it ends where the prefix stops matching.
        // Paths order by their levels, so everything under `path` sits directly
        // after it: a name that is not under it differs at a level `path` also
        // has, which puts the whole of that name's subtree past all of this
        // one's.
        let at = taken.partition_point(|c| c.path < *path);
        for other in taken[at..].iter().take_while(|c| c.path.starts_with(path)) {
            if other.by != by {
                return Err(refuse(other));
            }
        }

        taken.insert(
            at,
            Owner {
                path: path.clone(),
                by,
            },
        );
        Ok(())
    }

    /// The schema that took `path`, for a report or the inspector.
    pub fn declared_by(&self, path: &StorePath) -> Option<&'static str> {
        let taken = self.taken.read();
        taken
            .binary_search_by(|c| c.path.cmp(path))
            .ok()
            .map(|at| taken[at].by)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(joined: &str) -> StorePath {
        StorePath::parse_joined(joined).unwrap()
    }

    #[test]
    fn one_schema_taking_the_same_place_twice_takes_it_once() {
        let places = Places::default();

        places.take(&path("ui.theme"), "Ui").unwrap();
        places.take(&path("ui.theme"), "Ui").unwrap();

        assert_eq!(
            places.taken.read().len(),
            1,
            "reconstructing a struct must not pile the same place up"
        );
    }

    #[test]
    fn two_schemas_cannot_take_one_place() {
        let places = Places::default();
        places.take(&path("ui.theme"), "Ui").unwrap();

        let taken = places.take(&path("ui.theme"), "Theme").unwrap_err();
        let report = refused(&taken);

        let named: Vec<&Owner> = amethystate_core::facts::all(&report).collect();
        assert_eq!(named.len(), 2, "the report names both: {report:?}");
    }

    #[test]
    fn a_place_covers_what_is_under_it() {
        let places = Places::default();
        places.take(&path("widths"), "Columns").unwrap();

        let taken = places
            .take(&path("widths.left"), "Panels")
            .expect_err("a map owns its entries, so nobody else may put one there");

        assert_eq!(taken.held_at, path("widths"));
        assert_eq!(taken.held_by, "Columns");
        assert_eq!(taken.at, path("widths.left"));
        assert_eq!(taken.wanted_by, "Panels");
    }

    #[test]
    fn a_place_is_refused_by_one_already_inside_it() {
        let places = Places::default();
        places.take(&path("ui.panels.left"), "Panels").unwrap();

        let taken = places
            .take(&path("ui.panels"), "Ui")
            .expect_err("the outer one would take the level the inner one lives on");

        assert_eq!(taken.held_at, path("ui.panels.left"));
        assert_eq!(taken.held_by, "Panels");
    }

    #[test]
    fn a_level_may_be_shared_when_the_places_are_not() {
        let places = Places::default();

        places.take(&path("ui.accent"), "UiColors").unwrap();
        places
            .take(&path("ui.density"), "UiLayout")
            .expect("two schemas may sit on one level while owning different keys");
    }

    #[test]
    fn a_sibling_that_sorts_inside_the_run_does_not_hide_an_ancestor() {
        let places = Places::default();

        places.take(&path("ui"), "Ui").unwrap();
        places.take(&path("ui!x"), "Other").unwrap();

        let taken = places
            .take(&path("ui.theme"), "Theme")
            .expect_err("`ui` holds `ui.theme` and must refuse it");

        assert_eq!(
            taken.held_at,
            path("ui"),
            "`ui!x` sorts between `ui` and `ui.theme`, and a refusal naming it \
             would mean the walk stopped there instead of reaching the ancestor"
        );
        assert_eq!(taken.held_by, "Ui");
    }

    #[test]
    fn a_string_prefix_is_not_a_place() {
        let places = Places::default();
        places.take(&path("ui"), "Ui").unwrap();

        places
            .take(&path("uix.width"), "Uix")
            .expect("`ui` does not hold `uix.width`");
    }

    #[test]
    fn a_place_names_who_took_it() {
        let places = Places::default();
        places.take(&path("ui.theme"), "Ui").unwrap();

        assert_eq!(places.declared_by(&path("ui.theme")), Some("Ui"));
        assert_eq!(places.declared_by(&path("ui.other")), None);
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    fn a_level() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just("pot"),
            Just("potato"),
            Just("pot!luck"),
            Just("ato"),
            Just("lid"),
        ]
    }

    fn a_path() -> impl Strategy<Value = StorePath> {
        prop::collection::vec(a_level(), 1..4).prop_map(StorePath::from_segments)
    }

    fn an_owner() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("A"), Just("B")]
    }

    fn met_by<'a>(standing: &'a [Owner], path: &StorePath, by: &str) -> Option<&'a Owner> {
        standing
            .iter()
            .find(|other| other.by != by && path.overlaps(&other.path))
    }

    fn taken_by_the_rule(wanted: &[(StorePath, &'static str)]) -> Vec<Owner> {
        let mut standing: Vec<Owner> = Vec::new();

        for (path, by) in wanted {
            if met_by(&standing, path, by).is_some() {
                continue;
            }
            if standing.iter().any(|c| c.path == *path && c.by == *by) {
                continue;
            }
            standing.push(Owner {
                path: path.clone(),
                by,
            });
        }

        standing.sort_by(|a, b| a.path.cmp(&b.path));
        standing
    }

    proptest! {
        #[test]
        fn a_place_is_refused_exactly_when_it_meets_another_owners(
            wanted in prop::collection::vec((a_path(), an_owner()), 0..8)
        ) {
            let places = Places::default();
            let mut standing: Vec<Owner> = Vec::new();

            for (path, by) in &wanted {
                let expected = met_by(&standing, path, by).cloned();
                let answer = places.take(path, by);

                match (expected, answer.is_err()) {
                    (Some(other), true) => {
                        prop_assert!(
                            standing.contains(&other),
                            "refused by a place nobody took"
                        );
                    }
                    (None, false) => {
                        if !standing.iter().any(|c| c.path == *path && c.by == *by) {
                            standing.push(Owner { path: path.clone(), by });
                        }
                    }
                    (Some(other), false) => prop_assert!(
                        false,
                        "`{path}` was taken by `{by}` while `{other}` stands"
                    ),
                    (None, true) => prop_assert!(
                        false,
                        "`{path}` was refused for `{by}` and nothing stands over it: {:?}",
                        standing
                    ),
                }
            }
        }

        #[test]
        fn the_shortcut_and_the_walk_agree_whatever_the_order(
            wanted in prop::collection::vec((a_path(), an_owner()), 0..8)
        ) {
            let places = Places::default();
            for (path, by) in &wanted {
                let _ = places.take(path, by);
            }

            let mut held = places.taken.read().clone();
            held.sort_by(|a, b| a.path.cmp(&b.path));

            prop_assert_eq!(held, taken_by_the_rule(&wanted));
        }

        #[test]
        fn one_owner_may_take_any_set_of_places(
            paths in prop::collection::vec(a_path(), 0..8)
        ) {
            let places = Places::default();

            for path in &paths {
                prop_assert!(
                    places.take(path, "Alone").is_ok(),
                    "`{path}` was refused to the owner that already held it"
                );
            }
        }

        #[test]
        fn the_places_are_sorted_however_they_arrived(
            wanted in prop::collection::vec((a_path(), an_owner()), 0..12)
        ) {
            let places = Places::default();
            for (path, by) in &wanted {
                let _ = places.take(path, by);
            }

            let held = places.taken.read();
            let sorted: Vec<&StorePath> = {
                let mut at: Vec<&StorePath> = held.iter().map(|c| &c.path).collect();
                at.sort();
                at
            };

            prop_assert_eq!(
                held.iter().map(|c| &c.path).collect::<Vec<_>>(),
                sorted
            );
        }

        #[test]
        fn taking_a_place_twice_leaves_one(path in a_path()) {
            let places = Places::default();

            places.take(&path, "Twice").unwrap();
            places.take(&path, "Twice").unwrap();

            prop_assert_eq!(places.taken.read().len(), 1);
        }
    }
}
