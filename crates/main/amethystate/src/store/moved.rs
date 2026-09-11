//! What changed between the places a struct declared last time and the places
//! it declares now.
//!
//! The comparison is over the tree of declared places and nothing else. A
//! leaf's type is the user's, and a value that stops decoding is answered where
//! it is read - by `on_unreadable`, and listed by `disagreements()`. What is
//! judged here is where things sit.
//!
//! Named for what it looks for: data that is no longer under any declaration.

use crate::migration::fields::{FieldDescriptor, Role};
use crate::store::StorePath;
use crate::store::meta::{SchemaSnapshot, StoredFieldEntry};
use std::fmt;

/// One difference between the two trees.
#[derive(Debug, Clone, PartialEq)]
pub struct Moved {
    /// Where it is, under the struct's prefix.
    pub at: StorePath,
    pub what: What,
}

#[derive(Debug, Clone, PartialEq)]
pub enum What {
    /// A place nothing declared before. It takes the subtree beneath it, which
    /// was open until now.
    Taken,

    /// A place that was declared and is not any more. Whatever is stored there
    /// is out from under any declaration.
    Released,

    /// The same place, holding a different kind of thing.
    Role { was: Role, now: Role },

    /// A place that may now hold nothing where it could not, or the reverse.
    Optional { now: bool },
}

impl fmt::Display for Moved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = &self.at;
        match &self.what {
            What::Taken => write!(f, "`{at}` is declared now and was not before"),
            What::Released => write!(f, "`{at}` was declared before and is not now"),
            What::Role { was, now } => {
                write!(f, "`{at}` was {was:?} and is {now:?}")
            }
            What::Optional { now } => write!(f, "`{at}` may hold nothing: {now}"),
        }
    }
}

/// What one difference amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Data written by the old declaration is not under the new one. Somebody
    /// has to say what happens to it, which is what a migration step is.
    Breaks,

    /// A place was taken that nothing declared before. Whether that matters
    /// depends on whether anything is there, which this comparison cannot see:
    /// it knows declarations and not data. Over empty ground it is nothing at
    /// all - the commonest and most harmless change there is - and over
    /// occupied ground it is an annexation.
    LookAtTheGround,

    /// Neither. The places are where they were.
    Harmless,
}

impl Moved {
    pub fn verdict(&self) -> Verdict {
        match self.what {
            What::Released | What::Role { .. } => Verdict::Breaks,
            What::Taken => Verdict::LookAtTheGround,
            What::Optional { .. } => Verdict::Harmless,
        }
    }
}

/// Every difference between what was declared and what is declared: the places
/// that were held and are not, in the order they were declared, and then the
/// places newly taken.
///
/// Both sides are read down to the places they *own* and compared there, path
/// against path. A name pairs nothing: a declaration is where it lands, two
/// builds may spell one layout two ways, and the level a name is written at is
/// not always the level it lands at.
///
/// A node is therefore never named here. Nothing is stored at one,
/// [`Places`](crate::store::places::Places) is never asked to take one, and a
/// raw write beside its fields belongs to whoever wrote it: a leaf and a map
/// are places, a node is the way to them, and a flattened node is not even a
/// segment on the way.
///
/// ```text
///   declared                    owned
///   ------------------------------------------------
///   ui            node          -           a node is not a place
///     theme       leaf          ui.theme
///
///   ui            node flat     -           and lends no segment either
///     theme       leaf          theme
/// ```
///
/// So a node that gains or loses its segment is reported as what it does,
/// which is move every place beneath it: each old path released and each new
/// one claimed. Naming the node instead would name a path that holds nothing
/// and leave the caller to work out which places it meant.
///
/// A rename reads the same way - a [`What::Released`] beside a
/// [`What::Taken`], not to be told from the two of them happening at once,
/// which is what `#[rename(old => new)]` exists to say.
///
/// ```text
///   was          now              between them
///   ----------------------------------------------------------------
///   (nothing)    ui     node      `ui.theme` claimed    not `ui`
///                  theme
///
///   (nothing)    ui     flat      `theme` claimed       no segment either
///                  theme
///
///   (nothing)    open   map       `open` claimed        a map is a place,
///                                                       and takes open.*
///
///   ui           ui     flat      `ui.theme` released
///     theme        theme          `theme` claimed
/// ```
pub fn between(was: &[StoredFieldEntry], now: &[FieldDescriptor]) -> Vec<Moved> {
    let before = owned_stored(was);
    let after = owned(now);

    let mut met = vec![false; after.len()];
    let mut found = Vec::new();

    for old in &before {
        let Some(index) = after
            .iter()
            .enumerate()
            .find(|(index, new)| !met[*index] && new.at == old.at)
            .map(|(index, _)| index)
        else {
            found.push(Moved {
                at: old.at.clone(),
                what: What::Released,
            });
            continue;
        };

        met[index] = true;
        let new = &after[index];

        if old.role != new.role {
            found.push(Moved {
                at: old.at.clone(),
                what: What::Role {
                    was: old.role,
                    now: new.role,
                },
            });
        }

        if old.optional != new.optional {
            found.push(Moved {
                at: old.at.clone(),
                what: What::Optional { now: new.optional },
            });
        }
    }

    for (new, met) in after.iter().zip(met) {
        if !met {
            found.push(Moved {
                at: new.at.clone(),
                what: What::Taken,
            });
        }
    }

    found
}

/// The recorded tree that is the same declaration as `now`, if one is.
///
/// A declaration is the places it owns, so two trees are the same one when
/// they own a place in common. That is decidable rather than a guess: the
/// declarations at a prefix own disjoint places - [`Places`] refuses them
/// otherwise - so a place belongs to at most one of them on either side, and a
/// tree meets at most one tree.
///
/// Sharing nothing is not a puzzle either. A declaration whose every place
/// moved is a removal and an addition, which is what the two look like from
/// here and what they are: the data that was under the old places is out from
/// under any declaration, and the new places annexed whatever was under them.
///
/// [`Places`]: crate::store::places::Places
pub fn same_declaration(recorded: &[SchemaSnapshot], now: &[FieldDescriptor]) -> Option<usize> {
    let claimed = owned(now);

    recorded
        .iter()
        .position(|was| meet(&owned_stored(&was.fields), &claimed))
}

/// [`same_declaration`] between two recorded trees, which is the form
/// [`recording`] asks in: a snapshot about to be written is already stored
/// shape rather than what the code declares.
fn same_declaration_stored(recorded: &[SchemaSnapshot], now: &[StoredFieldEntry]) -> Option<usize> {
    let claimed = owned_stored(now);

    recorded
        .iter()
        .position(|was| meet(&owned_stored(&was.fields), &claimed))
}

/// What recording `schema` does to the snapshots already held at its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    /// The same declaration is recorded and says the same thing, so the file is
    /// not touched at all. This is what stops a rebuild of one struct writing.
    Unchanged,

    /// The same declaration is recorded and has moved, so it is replaced where
    /// it stands rather than added beside itself.
    Replacing(usize),

    /// Nothing recorded shares a place with it, so it joins them.
    Appending,
}

/// The one decision every engine makes when a struct records what it declares,
/// answered once rather than spelled out per engine.
///
/// Each engine is left its own half - how it reads the snapshots and how it
/// writes them back - which is where they genuinely differ.
pub fn recording(held: &[SchemaSnapshot], schema: &SchemaSnapshot) -> Recording {
    match same_declaration_stored(held, &schema.fields) {
        Some(index) if held[index] == *schema => Recording::Unchanged,
        Some(index) => Recording::Replacing(index),
        None => Recording::Appending,
    }
}

/// Puts `schema` where [`recording`] says it goes, answering whether anything
/// changed - and so whether the caller has to write.
pub fn record_into(held: &mut Vec<SchemaSnapshot>, schema: &SchemaSnapshot) -> bool {
    match recording(held, schema) {
        Recording::Unchanged => return false,
        Recording::Replacing(index) => held[index] = schema.clone(),
        Recording::Appending => held.push(schema.clone()),
    }

    true
}

fn meet(a: &[Place], b: &[Place]) -> bool {
    a.iter().any(|one| b.iter().any(|other| other.at == one.at))
}

/// Two declarations recorded at one version of a prefix, both owning the same
/// place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contradiction {
    pub version: u32,
    pub at: StorePath,
    pub between: (Option<String>, Option<String>),
}

impl fmt::Display for Contradiction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let named = |who: &Option<String>| who.clone().unwrap_or_else(|| "an unnamed one".into());

        write!(
            f,
            "two declarations are recorded at version {} and both own {}: {} and {}",
            self.version,
            self.at,
            named(&self.between.0),
            named(&self.between.1),
        )
    }
}

/// Whether what is recorded at one prefix says two things at once.
///
/// Declarations at a prefix own disjoint places, which is what makes a place
/// enough to tell one from another - see [`same_declaration`]. The recorded
/// list is a history rather than one moment, so that holds only inside a
/// version, and this is where it is required rather than assumed: a version
/// whose declarations claim a place in common is a set nothing can read back,
/// because the place belongs to whichever of them is looked at first.
pub fn contradiction(held: &[SchemaSnapshot]) -> Option<Contradiction> {
    for (index, one) in held.iter().enumerate() {
        let mine = owned_stored(&one.fields);

        for other in held.iter().skip(index + 1) {
            if other.version != one.version {
                continue;
            }

            let theirs = owned_stored(&other.fields);
            let Some(shared) = mine
                .iter()
                .find(|place| theirs.iter().any(|other| other.at == place.at))
            else {
                continue;
            };

            return Some(Contradiction {
                version: one.version,
                at: shared.at.clone(),
                between: (one.struct_name.clone(), other.struct_name.clone()),
            });
        }
    }

    None
}

/// A place a declaration owns, and what stands at it.
///
/// The whole of what the comparison reads: a path, and the two things about it
/// the store itself keeps - whether it holds one value or a level of entries,
/// and whether it may hold nothing.
struct Place {
    at: StorePath,
    role: Role,
    optional: bool,
}

/// Every place a set of declared fields owns, in the order they are declared.
///
/// A node owns none of its own - it is the way to the places below it - so it
/// contributes what is under it, at its own level or at its holder's when it
/// is flattened. A leaf and a map own their path and stop there: what is
/// inside a value, and what sits under a map's level, is theirs already.
fn owned(fields: &[FieldDescriptor]) -> Vec<Place> {
    fn walk(under: &StorePath, fields: &[FieldDescriptor], into: &mut Vec<Place>) {
        for field in fields {
            match field.owns(under) {
                Some(at) => into.push(Place {
                    at,
                    role: field.role,
                    optional: field.optional,
                }),
                None => walk(&field.below(under), field.children, into),
            }
        }
    }

    let mut found = Vec::new();
    walk(&StorePath::root(), fields, &mut found);
    found
}

/// The same, over what was written down.
fn owned_stored(fields: &[StoredFieldEntry]) -> Vec<Place> {
    fn walk(under: &StorePath, fields: &[StoredFieldEntry], into: &mut Vec<Place>) {
        for field in fields {
            let at = under.join(&field.name);

            match field.shape.role {
                Role::Node => {
                    let below = match field.shape.flattened {
                        true => under.clone(),
                        false => at,
                    };
                    walk(&below, &field.shape.children, into);
                }
                role => into.push(Place {
                    at,
                    role,
                    optional: field.shape.optional,
                }),
            }
        }
    }

    let mut found = Vec::new();
    walk(&StorePath::root(), fields, &mut found);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::StoredShape;

    fn stored(name: &str, shape: StoredShape) -> StoredFieldEntry {
        StoredFieldEntry {
            name: StorePath::parse_joined(name).unwrap(),
            type_name: "T".to_string(),
            shape,
        }
    }

    fn node(children: Vec<StoredFieldEntry>, flattened: bool) -> StoredShape {
        StoredShape {
            role: Role::Node,
            optional: false,
            children,
            flattened,
        }
    }

    const fn child(segments: &'static [&'static str], joined: &'static str) -> FieldDescriptor {
        FieldDescriptor::leaf(segments, joined, "T")
    }

    fn snapshot(version: u32, fields: Vec<StoredFieldEntry>) -> SchemaSnapshot {
        SchemaSnapshot {
            version,
            struct_name: Some("Ui".to_string()),
            fields,
        }
    }

    fn named(who: &str, version: u32, fields: Vec<StoredFieldEntry>) -> SchemaSnapshot {
        SchemaSnapshot {
            version,
            struct_name: Some(who.to_string()),
            fields,
        }
    }

    #[test]
    fn two_declarations_at_one_version_owning_a_place_in_common_is_a_contradiction() {
        let held = vec![
            named("Ui", 2, vec![stored("theme", StoredShape::field())]),
            named(
                "Panels",
                2,
                vec![
                    stored("theme", StoredShape::field()),
                    stored("left", StoredShape::field()),
                ],
            ),
        ];

        let said = contradiction(&held).expect("both own `theme` at version 2");

        assert_eq!(said.version, 2);
        assert_eq!(said.at, StorePath::segment("theme"));
        assert_eq!(
            said.between,
            (Some("Ui".to_string()), Some("Panels".to_string()))
        );
    }

    #[test]
    fn the_same_place_at_two_versions_is_the_history_it_is_meant_to_be() {
        let held = vec![
            named("Ui", 1, vec![stored("theme", StoredShape::field())]),
            named("Ui", 2, vec![stored("theme", StoredShape::field())]),
        ];

        assert_eq!(contradiction(&held), None);
    }

    #[test]
    fn two_declarations_at_one_version_owning_nothing_in_common_stand_together() {
        let held = vec![
            named("Ui", 2, vec![stored("theme", StoredShape::field())]),
            named("Panels", 2, vec![stored("left", StoredShape::field())]),
        ];

        assert_eq!(contradiction(&held), None);
    }

    #[test]
    fn recording_what_is_already_recorded_writes_nothing() {
        let one = snapshot(1, vec![stored("theme", StoredShape::field())]);
        let mut held = vec![one.clone()];

        assert_eq!(recording(&held, &one), Recording::Unchanged);
        assert!(!record_into(&mut held, &one));
        assert_eq!(held.len(), 1);
    }

    #[test]
    fn a_declaration_that_changed_replaces_the_one_recorded_for_it() {
        let held_at_v1 = snapshot(1, vec![stored("theme", StoredShape::field())]);
        let mut held = vec![held_at_v1];

        let now = snapshot(
            2,
            vec![
                stored("theme", StoredShape::field()),
                stored("scale", StoredShape::field()),
            ],
        );

        assert_eq!(recording(&held, &now), Recording::Replacing(0));
        assert!(record_into(&mut held, &now));
        assert_eq!(held, vec![now], "it stands where it stood, not beside itself");
    }

    #[test]
    fn a_declaration_sharing_no_place_joins_the_rest() {
        let theirs = snapshot(1, vec![stored("theme", StoredShape::field())]);
        let mut held = vec![theirs.clone()];

        let ours = snapshot(1, vec![stored("host", StoredShape::field())]);

        assert_eq!(recording(&held, &ours), Recording::Appending);
        assert!(record_into(&mut held, &ours));
        assert_eq!(held, vec![theirs, ours]);
    }

    #[test]
    fn the_same_tree_twice_has_not_moved() {
        static NOW: &[FieldDescriptor] = &[child(&["theme"], "theme"), child(&["scale"], "scale")];

        let was = vec![
            stored("theme", StoredShape::field()),
            stored("scale", StoredShape::field()),
        ];

        assert_eq!(between(&was, NOW), []);
    }

    #[test]
    fn a_rename_reads_as_a_release_beside_a_claim() {
        static NOW: &[FieldDescriptor] = &[child(&["nickname"], "nickname")];

        let was = vec![stored("handle", StoredShape::field())];

        assert_eq!(
            between(&was, NOW),
            [
                Moved {
                    at: StorePath::segment("handle"),
                    what: What::Released
                },
                Moved {
                    at: StorePath::segment("nickname"),
                    what: What::Taken
                },
            ]
        );
    }

    #[test]
    fn a_leaf_that_became_a_map_breaks() {
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Map,
            ..child(&["open"], "open")
        }];

        let was = vec![stored("open", StoredShape::field())];

        let found = between(&was, NOW);

        assert_eq!(
            found,
            [Moved {
                at: StorePath::segment("open"),
                what: What::Role {
                    was: Role::Field,
                    now: Role::Map
                }
            }]
        );
        assert_eq!(found[0].verdict(), Verdict::Breaks);
    }

    #[test]
    fn a_node_that_lost_its_segment_moves_every_place_beneath_it() {
        static UNDER: &[FieldDescriptor] = &[child(&["theme"], "theme")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            flattened: true,
            ..child(&["ui"], "ui")
        }];

        let was = vec![stored(
            "ui",
            node(vec![stored("theme", StoredShape::field())], false),
        )];

        let found = between(&was, NOW);

        assert_eq!(
            found,
            [
                Moved {
                    at: StorePath::from_segments(["ui", "theme"]),
                    what: What::Released
                },
                Moved {
                    at: StorePath::segment("theme"),
                    what: What::Taken
                },
            ],
            "nothing was ever stored at `ui`, and the place that did hold \
             something is the one that moved"
        );
        assert_eq!(found[0].verdict(), Verdict::Breaks);
    }

    #[test]
    fn a_place_under_a_node_is_named_by_its_whole_path() {
        static UNDER: &[FieldDescriptor] = &[child(&["scale"], "scale")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            ..child(&["ui"], "ui")
        }];

        let was = vec![stored(
            "ui",
            node(vec![stored("theme", StoredShape::field())], false),
        )];

        assert_eq!(
            between(&was, NOW),
            [
                Moved {
                    at: StorePath::from_segments(["ui", "theme"]),
                    what: What::Released
                },
                Moved {
                    at: StorePath::from_segments(["ui", "scale"]),
                    what: What::Taken
                },
            ]
        );
    }

    #[test]
    fn a_flattened_node_lends_its_children_no_segment() {
        static UNDER: &[FieldDescriptor] = &[child(&["scale"], "scale")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            flattened: true,
            ..child(&["ui"], "ui")
        }];

        let was = vec![stored(
            "ui",
            node(vec![stored("theme", StoredShape::field())], true),
        )];

        assert_eq!(
            between(&was, NOW),
            [
                Moved {
                    at: StorePath::segment("theme"),
                    what: What::Released
                },
                Moved {
                    at: StorePath::segment("scale"),
                    what: What::Taken
                },
            ]
        );
    }

    #[test]
    fn a_place_that_may_now_hold_nothing_is_harmless() {
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            optional: true,
            ..child(&["nickname"], "nickname")
        }];

        let was = vec![stored("nickname", StoredShape::field())];

        let found = between(&was, NOW);

        assert_eq!(
            found,
            [Moved {
                at: StorePath::segment("nickname"),
                what: What::Optional { now: true }
            }]
        );
        assert_eq!(found[0].verdict(), Verdict::Harmless);
    }

    #[test]
    fn a_node_is_not_a_place_so_what_is_claimed_is_under_it() {
        static UNDER: &[FieldDescriptor] = &[child(&["theme"], "theme")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            ..child(&["ui"], "ui")
        }];

        assert_eq!(
            between(&[], NOW),
            [Moved {
                at: StorePath::from_segments(["ui", "theme"]),
                what: What::Taken
            }],
            "nothing is stored at `ui`, and a write beside `ui.theme` is not this struct's"
        );
    }

    #[test]
    fn a_map_is_a_place_and_takes_everything_under_it() {
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Map,
            ..child(&["open"], "open")
        }];

        assert_eq!(
            between(&[], NOW),
            [Moved {
                at: StorePath::segment("open"),
                what: What::Taken
            }]
        );
    }

    #[test]
    fn a_flattened_node_is_claimed_as_what_it_brings() {
        static UNDER: &[FieldDescriptor] = &[child(&["theme"], "theme")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            flattened: true,
            ..child(&["ui"], "ui")
        }];

        assert_eq!(
            between(&[], NOW),
            [Moved {
                at: StorePath::segment("theme"),
                what: What::Taken
            }]
        );
    }

    #[test]
    fn a_flattened_node_that_is_gone_releases_what_it_brought() {
        let was = vec![stored(
            "ui",
            node(vec![stored("theme", StoredShape::field())], true),
        )];

        assert_eq!(
            between(&was, &[]),
            [Moved {
                at: StorePath::segment("theme"),
                what: What::Released
            }]
        );
    }

    #[test]
    fn a_node_flattened_on_both_sides_says_nothing_about_itself() {
        static UNDER: &[FieldDescriptor] = &[child(&["theme"], "theme")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            flattened: true,
            optional: true,
            ..child(&["ui"], "ui")
        }];

        let was = vec![stored(
            "ui",
            node(vec![stored("theme", StoredShape::field())], true),
        )];

        assert_eq!(between(&was, NOW), []);
    }

    #[test]
    fn a_leaf_reached_where_a_node_stood_takes_ground_nobody_held() {
        static INNER: &[FieldDescriptor] = &[child(&["a"], "a")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: INNER,
            flattened: true,
            ..child(&["a"], "a")
        }];

        let was = vec![stored("a", node(vec![], false))];

        let found = between(&was, NOW);

        assert_eq!(
            found,
            [Moved {
                at: StorePath::segment("a"),
                what: What::Taken
            }],
            "the old `a` was a node holding nothing, so nothing was released"
        );
        assert_eq!(found[0].verdict(), Verdict::LookAtTheGround);
    }

    #[test]
    fn a_leaf_that_became_a_node_releases_its_place_and_claims_what_is_under_it() {
        static UNDER: &[FieldDescriptor] = &[child(&["theme"], "theme")];
        static NOW: &[FieldDescriptor] = &[FieldDescriptor {
            role: Role::Node,
            children: UNDER,
            ..child(&["ui"], "ui")
        }];

        let was = vec![stored("ui", StoredShape::field())];

        let found = between(&was, NOW);

        assert_eq!(
            found,
            [
                Moved {
                    at: StorePath::segment("ui"),
                    what: What::Released
                },
                Moved {
                    at: StorePath::from_segments(["ui", "theme"]),
                    what: What::Taken
                },
            ]
        );
        assert_eq!(found[0].verdict(), Verdict::Breaks);
    }

    #[test]
    fn every_kind_of_move_has_its_verdict() {
        let moved = |what| Moved {
            at: StorePath::segment("x"),
            what,
        };

        assert_eq!(moved(What::Released).verdict(), Verdict::Breaks);
        assert_eq!(
            moved(What::Role {
                was: Role::Field,
                now: Role::Map
            })
            .verdict(),
            Verdict::Breaks
        );
        assert_eq!(moved(What::Taken).verdict(), Verdict::LookAtTheGround);
        assert_eq!(
            moved(What::Optional { now: true }).verdict(),
            Verdict::Harmless
        );
        assert_eq!(
            moved(What::Optional { now: false }).verdict(),
            Verdict::Harmless
        );
    }

    #[test]
    fn a_store_opened_for_the_first_time_has_nothing_to_have_moved_from() {
        static NOW: &[FieldDescriptor] = &[child(&["theme"], "theme")];

        let found = between(&[], NOW);

        assert_eq!(
            found,
            [Moved {
                at: StorePath::segment("theme"),
                what: What::Taken
            }]
        );
        assert_eq!(found[0].verdict(), Verdict::LookAtTheGround);
    }
}

/// One declaration tree, generated, and the two forms `between` compares.
///
/// The two sides of the comparison are different types - one read off a disk,
/// one written by the macro - so a property needs a third thing that becomes
/// either. That is [`Decl`]: a tree is drawn once and projected twice, which is
/// what makes "the same tree twice" a statement a generator can make.
#[cfg(test)]
mod properties {
    use super::*;
    use crate::store::StaticPath;
    use crate::store::meta::StoredShape;
    use proptest::prelude::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Decl {
        /// The levels this declaration's name is, which `path = "a.b"` makes
        /// more than one of.
        levels: Vec<String>,
        role: Role,
        optional: bool,
        flattened: bool,
        children: Vec<Decl>,
    }

    impl Decl {
        fn name(&self) -> StorePath {
            StorePath::from_segments(&self.levels)
        }
    }

    /// The tree as the disk holds it.
    fn as_stored(at: &Decl) -> StoredFieldEntry {
        StoredFieldEntry {
            name: at.name(),
            type_name: "T".to_string(),
            shape: StoredShape {
                role: at.role,
                optional: at.optional,
                flattened: at.flattened,
                children: at.children.iter().map(as_stored).collect(),
            },
        }
    }

    /// The tree as the macro writes it.
    ///
    /// Leaked, because a descriptor's children are `&'static` - the macro
    /// writes them into the binary and nothing frees them there either. A
    /// generated case leaks a few hundred bytes and the test process exits.
    fn as_declared(at: &Decl) -> FieldDescriptor {
        let segments: &'static [&'static str] = Box::leak(
            at.levels
                .iter()
                .map(|level| &*Box::leak(level.clone().into_boxed_str()))
                .collect::<Box<[&'static str]>>(),
        );
        let joined: &'static str =
            Box::leak(StorePath::from_segments(&at.levels).to_string().into_boxed_str());

        FieldDescriptor {
            name: StaticPath::new(segments, joined),
            declared: joined,
            type_name: "T",
            role: at.role,
            optional: at.optional,
            flattened: at.flattened,
            children: Box::leak(at.children.iter().map(as_declared).collect::<Box<[_]>>()),
        }
    }

    fn all_declared(tree: &[Decl]) -> &'static [FieldDescriptor] {
        Box::leak(tree.iter().map(as_declared).collect::<Box<[_]>>())
    }

    /// Every place the tree declares, worked out without going near `between`.
    ///
    /// This is the oracle the properties are held against, so it says the rule
    /// in the other direction: a flattened node lends its children no segment,
    /// and everything else adds its own.
    fn places(tree: &[Decl], under: &StorePath, into: &mut Vec<StorePath>) {
        for one in tree {
            let at = under.join(&one.name());

            match one.flattened {
                true => places(&one.children, under, into),
                false => {
                    into.push(at.clone());
                    places(&one.children, &at, into);
                }
            }
        }
    }

    fn declared_places(tree: &[Decl]) -> Vec<StorePath> {
        let mut found = Vec::new();
        places(tree, &StorePath::root(), &mut found);
        found
    }

    /// The places and what stands at each, which is what "the same tree" has to
    /// mean: two trees can declare one set of paths and hold different things
    /// there - a node at `a`, against a leaf reached at `a` through a flattened
    /// wrapper.
    fn shapes(tree: &[Decl], under: &StorePath, into: &mut Vec<(StorePath, Role, bool)>) {
        for one in tree {
            let at = under.join(&one.name());

            match one.flattened {
                true => shapes(&one.children, under, into),
                false => {
                    into.push((at.clone(), one.role, one.optional));
                    shapes(&one.children, &at, into);
                }
            }
        }
    }

    fn declared_shapes(tree: &[Decl]) -> Vec<(StorePath, Role, bool)> {
        let mut found = Vec::new();
        shapes(tree, &StorePath::root(), &mut found);
        found
    }

    /// The places the tree owns: what a claim names.
    ///
    /// Not every path it declares - a node is a way to the paths under it and
    /// is owned by nobody, which is the rule `Places` and `Kv` both keep. So
    /// this descends through nodes and stops at leaves and maps, which is what
    /// `Places::take` is called for and nothing else.
    fn claims(tree: &[Decl], under: &StorePath, into: &mut Vec<StorePath>) {
        for one in tree {
            let at = under.join(&one.name());

            match one.role {
                Role::Node => {
                    let below = if one.flattened { under } else { &at };
                    claims(&one.children, below, into);
                }
                Role::Field | Role::Map => into.push(at),
            }
        }
    }

    fn claimed_places(tree: &[Decl]) -> Vec<StorePath> {
        let mut found = Vec::new();
        claims(tree, &StorePath::root(), &mut found);
        found
    }

    /// The two trees and what came back, as lines a person reads.
    ///
    /// A `Vec<Moved>` in a `Debug` dump says nothing about which places existed
    /// to begin with, and that is exactly what a failure here is about.
    fn shown(was: &[Decl], now: &[Decl], moved: &[Moved]) -> String {
        let list = |tree: &[Decl]| match declared_places(tree) {
            places if places.is_empty() => "    (nothing)".to_string(),
            places => places
                .iter()
                .map(|at| format!("    {at}"))
                .collect::<Vec<_>>()
                .join("\n"),
        };

        let said = match moved.is_empty() {
            true => "    (nothing moved)".to_string(),
            false => moved
                .iter()
                .map(|one| format!("    {:?}  {one}", one.verdict()))
                .collect::<Vec<_>>()
                .join("\n"),
        };

        format!(
            "\nwas:\n{}\nnow:\n{}\nbetween them:\n{}\n",
            list(was),
            list(now),
            said
        )
    }

    fn a_name() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(
            prop_oneof![
                6 => prop::char::range('a', 'c').prop_map(|c| c.to_string()),
                2 => Just("dark.mode".to_string()),
                1 => Just("a\\b".to_string()),
            ],
            1..3,
        )
    }

    fn a_role() -> impl Strategy<Value = Role> {
        prop_oneof![Just(Role::Field), Just(Role::Map), Just(Role::Node)]
    }

    /// Siblings with one name are not a tree any declaration could be: the
    /// macro refuses two fields at one path, and a comparison that met them
    /// would answer about the first and count both.
    fn distinct(mut siblings: Vec<Decl>) -> Vec<Decl> {
        let mut seen = Vec::new();
        siblings.retain(|one| {
            let name = one.name();
            match seen.contains(&name) {
                true => false,
                false => {
                    seen.push(name);
                    true
                }
            }
        });
        siblings
    }

    /// A tree of declarations, shallow and narrow on purpose: what these
    /// properties are about is how levels compose, and depth past three adds
    /// running time rather than cases.
    fn a_tree() -> impl Strategy<Value = Vec<Decl>> {
        let leaf = (a_name(), a_role(), any::<bool>()).prop_map(|(levels, role, optional)| Decl {
            levels,
            role,
            optional,
            flattened: false,
            children: Vec::new(),
        });

        let one = leaf.prop_recursive(3, 24, 3, |inner| {
            (
                a_name(),
                any::<bool>(),
                any::<bool>(),
                prop::collection::vec(inner, 0..3),
            )
                .prop_map(|(levels, optional, flattened, children)| Decl {
                    levels,
                    role: Role::Node,
                    optional,
                    flattened,
                    children: distinct(children),
                })
        });

        prop::collection::vec(one, 0..4).prop_map(distinct)
    }

    proptest! {
        #[test]
        fn the_same_tree_twice_has_moved_nothing(tree in a_tree()) {
            let was: Vec<_> = tree.iter().map(as_stored).collect();
            let moved = between(&was, all_declared(&tree));

            prop_assert!(moved.is_empty(), "{}", shown(&tree, &tree, &moved));
        }

        #[test]
        fn every_place_reported_is_one_of_the_two_trees(was in a_tree(), now in a_tree()) {
            let stored: Vec<_> = was.iter().map(as_stored).collect();
            let moved = between(&stored, all_declared(&now));

            let mut known = declared_places(&was);
            known.extend(declared_places(&now));

            for one in &moved {
                prop_assert!(
                    known.contains(&one.at),
                    "`{}` is in neither tree{}",
                    one.at,
                    shown(&was, &now, &moved)
                );
            }
        }

        #[test]
        fn release_and_claim_are_mirror_images(a in a_tree(), b in a_tree()) {
            let stored_a: Vec<_> = a.iter().map(as_stored).collect();
            let stored_b: Vec<_> = b.iter().map(as_stored).collect();

            let forward = between(&stored_a, all_declared(&b));
            let backward = between(&stored_b, all_declared(&a));

            let taken = |moved: &[Moved], what: What| {
                let mut at: Vec<String> = moved
                    .iter()
                    .filter(|one| one.what == what)
                    .map(|one| one.at.to_string())
                    .collect();
                at.sort();
                at
            };

            prop_assert_eq!(
                taken(&forward, What::Released),
                taken(&backward, What::Taken),
                "{}", shown(&a, &b, &forward)
            );
            prop_assert_eq!(
                taken(&forward, What::Taken),
                taken(&backward, What::Released),
                "{}", shown(&a, &b, &forward)
            );
        }

        #[test]
        fn against_nothing_a_tree_claims_its_shallowest_places(tree in a_tree()) {
            let moved = between(&[], all_declared(&tree));

            let claimed: Vec<StorePath> = moved
                .iter()
                .filter(|one| one.what == What::Taken)
                .map(|one| one.at.clone())
                .collect();

            prop_assert_eq!(claimed.len(), moved.len(), "{}", shown(&[], &tree, &moved));
            prop_assert_eq!(
                claimed,
                claimed_places(&tree),
                "{}", shown(&[], &tree, &moved)
            );
        }

        #[test]
        fn against_nothing_declared_every_place_is_released(tree in a_tree()) {
            let was: Vec<_> = tree.iter().map(as_stored).collect();
            let moved = between(&was, &[]);

            let released: Vec<StorePath> = moved
                .iter()
                .filter(|one| one.what == What::Released)
                .map(|one| one.at.clone())
                .collect();

            prop_assert_eq!(released.len(), moved.len(), "{}", shown(&tree, &[], &moved));

            for one in &moved {
                prop_assert_eq!(
                    one.verdict(),
                    Verdict::Breaks,
                    "{}", shown(&tree, &[], &moved)
                );
            }
        }

        #[test]
        fn a_break_means_the_trees_differ(was in a_tree(), now in a_tree()) {
            let stored: Vec<_> = was.iter().map(as_stored).collect();
            let moved = between(&stored, all_declared(&now));

            if moved.iter().any(|one| one.verdict() == Verdict::Breaks) {
                prop_assert_ne!(
                    declared_shapes(&was),
                    declared_shapes(&now),
                    "something broke between two trees holding the same things{}",
                    shown(&was, &now, &moved)
                );
            }
        }
    }
}
