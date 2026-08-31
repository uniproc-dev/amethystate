use crate::store::error::{StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::Report;
use parking_lot::RwLock;
use std::fmt;

/// A stored path and the schema that claimed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub path: StorePath,
    pub by: &'static str,
}

impl fmt::Display for Claimed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} claims {}", self.by, self.path)
    }
}

/// Who owns what, so that two owners cannot write over each other.
///
/// A claim is made where a path is composed - the constructor of a field or a
/// map - and belongs to the *name* that made it. So the same schema claiming
/// the same path again is a no-op, reconstructing a struct in one process
/// works, and a claim stands for the life of the process.
#[derive(Default)]
pub struct Owners {
    claims: RwLock<Vec<Claimed>>,
}

fn refused(standing: &Claimed, path: &StorePath, by: &'static str) -> Report<StorageError> {
    Report::new(StorageError::Claimed)
        .attach(standing.clone())
        .attach(Claimed {
            path: path.clone(),
            by,
        })
}

impl Owners {
    /// Records that `by` owns `path` and everything under it, or refuses.
    pub fn claim(&self, path: &StorePath, by: &'static str) -> StorageResult<()> {
        let mut claims = self.claims.write();

        // A claim that holds `path` sits at `path` itself or at one of its
        // ancestors, since containment is a prefix at a level boundary. They
        // are looked up rather than scanned back to, because the run between an
        // ancestor and `path` is not all ancestors: `ui!x` sorts between `ui`
        // and `ui.theme`, so a walk that stops at the first non-prefix stops
        // before it reaches `ui`.
        let mut above = Some(path.clone());
        while let Some(one) = above {
            if let Ok(found) = claims.binary_search_by(|c| c.path.as_str().cmp(one.as_str())) {
                let other = &claims[found];
                if other.by != by {
                    return Err(refused(other, path, by));
                }
                if other.path == *path {
                    return Ok(());
                }
            }
            above = one.parent();
        }

        // Downwards it is a run: everything under `path` begins with it, and
        // begins with it contiguously. `overlaps` then separates a descendant
        // from a name that merely shares the characters - `ui.themex` starts
        // with `ui.theme` and is not under it.
        let at = claims.partition_point(|c| c.path.as_str() < path.as_str());
        for other in claims[at..]
            .iter()
            .take_while(|c| c.path.as_str().starts_with(path.as_str()))
        {
            if other.by != by && path.overlaps(&other.path) {
                return Err(refused(other, path, by));
            }
        }

        claims.insert(at, Claimed {
            path: path.clone(),
            by,
        });
        Ok(())
    }

    /// The schema that claimed `path`, for a report or the inspector.
    pub fn declared_by(&self, path: &StorePath) -> Option<&'static str> {
        let claims = self.claims.read();
        claims
            .binary_search_by(|c| c.path.as_str().cmp(path.as_str()))
            .ok()
            .map(|at| claims[at].by)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(joined: &str) -> StorePath {
        StorePath::parse_joined(joined).unwrap()
    }

    #[test]
    fn one_schema_claiming_the_same_path_twice_is_the_same_claim() {
        let owners = Owners::default();

        owners.claim(&path("ui.theme"), "Ui").unwrap();
        owners.claim(&path("ui.theme"), "Ui").unwrap();

        assert_eq!(
            owners.claims.read().len(),
            1,
            "reconstructing a struct must not pile up claims"
        );
    }

    #[test]
    fn two_schemas_cannot_claim_one_path() {
        let owners = Owners::default();
        owners.claim(&path("ui.theme"), "Ui").unwrap();

        let refused = owners.claim(&path("ui.theme"), "Theme").unwrap_err();

        let named: Vec<&Claimed> = amethystate_core::facts::all(&refused).collect();
        assert_eq!(named.len(), 2, "the report names both: {refused:?}");
    }

    #[test]
    fn a_claim_covers_what_is_under_it() {
        let owners = Owners::default();
        owners.claim(&path("widths"), "Columns").unwrap();

        assert!(
            owners.claim(&path("widths.left"), "Panels").is_err(),
            "a map owns its entries, so nobody else may put one there"
        );
    }

    #[test]
    fn a_claim_is_refused_by_one_already_inside_it() {
        let owners = Owners::default();
        owners.claim(&path("ui.panels.left"), "Panels").unwrap();

        assert!(
            owners.claim(&path("ui.panels"), "Ui").is_err(),
            "the outer one would take the level the inner one lives on"
        );
    }

    #[test]
    fn a_level_may_be_shared_when_the_claims_are_not() {
        let owners = Owners::default();

        owners.claim(&path("ui.accent"), "UiColors").unwrap();
        owners
            .claim(&path("ui.density"), "UiLayout")
            .expect("two schemas may sit on one level while owning different keys");
    }

    #[test]
    fn a_sibling_that_sorts_inside_the_run_does_not_hide_an_ancestor() {
        let owners = Owners::default();

        owners.claim(&path("ui"), "Ui").unwrap();
        owners.claim(&path("ui!x"), "Other").unwrap();

        assert!(
            owners.claim(&path("ui.theme"), "Theme").is_err(),
            "`ui` holds `ui.theme`, and `ui!x` sitting between them must not hide it"
        );
    }

    #[test]
    fn a_string_prefix_is_not_a_claim() {
        let owners = Owners::default();
        owners.claim(&path("ui"), "Ui").unwrap();

        owners
            .claim(&path("uix.width"), "Uix")
            .expect("`ui` does not hold `uix.width`");
    }

    #[test]
    fn a_claim_names_who_made_it() {
        let owners = Owners::default();
        owners.claim(&path("ui.theme"), "Ui").unwrap();

        assert_eq!(owners.declared_by(&path("ui.theme")), Some("Ui"));
        assert_eq!(owners.declared_by(&path("ui.other")), None);
    }
}
