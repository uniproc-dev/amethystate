//! Drift, laid out the way a compiler lays out a warning.
//!
//! The report already holds everything this shows - which prefix drifted, every
//! place under it that moved, and what each move amounts to. What is here is
//! the arrangement: one heading per prefix, the places under it as its own
//! nested diagnostics, and a severity per place so the one that breaks reads
//! differently from the one that is only worth a look.
//!
//! Nothing here decides anything. A place is `Breaks` or it is not before this
//! module sees it - see [`Verdict`] - and turning that off would be turning off
//! the check rather than the printing, which is why the check is not behind the
//! feature this module is.

use crate::migration::fields::Role;
use crate::migration::{MigrationReport, NaggingRecord};
use crate::store::moved::{Moved, Verdict, What};
use amethystate_core::path::StorePath;
use miette::Diagnostic;
use thiserror::Error;

/// One prefix holding a shape the code does not declare.
#[derive(Debug, Error, Diagnostic)]
#[error("`{prefix}` holds a shape this build does not declare")]
#[diagnostic(
    code(amethystate::drift),
    severity(Warning),
    help(
        "raise the struct's `version` and write a step for it, or declare the released places again"
    )
)]
pub struct Drift {
    prefix: StorePath,

    #[related]
    places: Vec<Place>,
}

/// One place under it, and what moving it costs.
#[derive(Debug, Error, Diagnostic)]
pub enum Place {
    #[error("`{at}` was declared before and is not now")]
    #[diagnostic(
        severity(Warning),
        help("what an earlier build wrote there is under no declaration now")
    )]
    Released { at: StorePath },

    #[error("`{at}` was written as {was:?} and is declared as {now:?}")]
    #[diagnostic(
        severity(Warning),
        help("one value and a level of entries are not the same thing on disk")
    )]
    Role { at: StorePath, was: Role, now: Role },

    #[error("`{at}` is declared now and was not before")]
    #[diagnostic(
        severity(Advice),
        help("nothing at all over empty ground, an annexation over occupied ground")
    )]
    Taken { at: StorePath },

    #[error("`{at}` may hold nothing now, where it could not before")]
    #[diagnostic(severity(Advice))]
    MayBeEmpty { at: StorePath },

    #[error("`{at}` may no longer hold nothing")]
    #[diagnostic(
        severity(Advice),
        help("what was written there as nothing reads back as the declared default")
    )]
    MustBeFilled { at: StorePath },
}

impl Place {
    fn of(under: &StorePath, moved: &Moved) -> Self {
        let at = under.join(&moved.at);

        match moved.what {
            What::Released => Place::Released { at },
            What::Role { was, now } => Place::Role { at, was, now },
            What::Taken => Place::Taken { at },
            What::Optional { now: true } => Place::MayBeEmpty { at },
            What::Optional { now: false } => Place::MustBeFilled { at },
        }
    }
}

impl Drift {
    /// The places in the order they read best: what breaks first, then what is
    /// worth a look, then the rest.
    ///
    /// The record carries them in the order the comparison walked, which is the
    /// declaration order of whichever tree it was walking - true to the code and
    /// no use to somebody deciding whether to act. What raised the complaint
    /// goes at the top instead, and the harmless moves stay because a change is
    /// easier to recognise whole than through the one part of it that failed.
    fn of(record: &NaggingRecord) -> Self {
        let mut moved: Vec<&Moved> = record.moved.iter().collect();
        moved.sort_by_key(|one| match one.verdict() {
            Verdict::Breaks => 0,
            Verdict::LookAtTheGround => 1,
            Verdict::Harmless => 2,
        });

        Self {
            prefix: record.prefix.clone(),
            places: moved
                .into_iter()
                .map(|one| Place::of(&record.prefix, one))
                .collect(),
        }
    }
}

impl MigrationReport {
    /// The drift in this report, one diagnostic per prefix that has any.
    ///
    /// Empty when nothing drifted, which is the same answer
    /// [`has_drift`](MigrationReport::has_drift) gives.
    pub fn drift(&self) -> Vec<miette::Report> {
        self.components
            .iter()
            .flat_map(|component| component.nagging.iter())
            .map(|record| miette::Report::new(Drift::of(record)))
            .collect()
    }
}

/// One diagnostic as it reaches a terminal.
pub(crate) fn rendered(report: &miette::Report) -> String {
    format!("{report:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::SchemaDiff;

    fn at(name: &str) -> StorePath {
        StorePath::parse_joined(name).unwrap()
    }

    fn record(moved: Vec<Moved>) -> NaggingRecord {
        NaggingRecord {
            prefix: at("app"),
            id: None,
            diff: None,
            moved,
        }
    }

    fn moved(name: &str, what: What) -> Moved {
        Moved { at: at(name), what }
    }

    fn drawn(report: &miette::Report) -> String {
        let mut out = String::new();
        miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor())
            .with_width(100)
            .render_report(&mut out, &**report)
            .unwrap();
        out
    }

    #[test]
    fn a_place_is_named_from_the_root_and_not_from_the_prefix() {
        let drift = Drift::of(&record(vec![moved("token", What::Released)]));

        assert!(
            matches!(&drift.places[0], Place::Released { at } if at.to_string() == "app.token"),
            "{:?}",
            drift.places
        );
    }

    #[test]
    fn what_breaks_is_read_first() {
        let drift = Drift::of(&record(vec![
            moved("scale", What::Optional { now: true }),
            moved("theme", What::Taken),
            moved("token", What::Released),
        ]));

        let named: Vec<String> = drift.places.iter().map(|place| place.to_string()).collect();

        assert_eq!(
            named,
            [
                "`app.token` was declared before and is not now",
                "`app.theme` is declared now and was not before",
                "`app.scale` may hold nothing now, where it could not before",
            ]
        );
    }

    #[test]
    fn a_break_is_a_warning_and_a_claim_is_advice() {
        let drift = Drift::of(&record(vec![
            moved("token", What::Released),
            moved("theme", What::Taken),
        ]));

        assert_eq!(drift.severity(), Some(miette::Severity::Warning));
        assert_eq!(
            drift.places[0].severity(),
            Some(miette::Severity::Warning),
            "a released place"
        );
        assert_eq!(
            drift.places[1].severity(),
            Some(miette::Severity::Advice),
            "a claimed place"
        );
    }

    #[test]
    fn a_report_with_no_drift_renders_nothing() {
        let report = MigrationReport::default();

        assert!(report.drift().is_empty());
        assert!(!report.has_drift());
    }

    #[test]
    fn every_prefix_that_drifted_gets_its_own_diagnostic() {
        let report = MigrationReport {
            components: vec![
                crate::migration::ComponentResult {
                    prefixes: vec![at("app")],
                    outcome: crate::migration::ComponentOutcome::Skipped(
                        crate::migration::NotMigrated::UpToDate,
                    ),
                    nagging: vec![record(vec![moved("token", What::Released)])],
                },
                crate::migration::ComponentResult {
                    prefixes: vec![at("ui")],
                    outcome: crate::migration::ComponentOutcome::Skipped(
                        crate::migration::NotMigrated::UpToDate,
                    ),
                    nagging: vec![NaggingRecord {
                        prefix: at("ui"),
                        id: None,
                        diff: Some(SchemaDiff {
                            added: vec![],
                            removed: vec![],
                        }),
                        moved: vec![moved("theme", What::Released)],
                    }],
                },
            ],
        };

        insta::assert_snapshot!(
            report
                .drift()
                .iter()
                .map(drawn)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn the_rendering_carries_the_places_under_the_prefix() {
        let drift = miette::Report::new(Drift::of(&record(vec![
            moved("token", What::Released),
            moved("theme", What::Taken),
        ])));

        insta::assert_snapshot!(drawn(&drift));
    }
}
