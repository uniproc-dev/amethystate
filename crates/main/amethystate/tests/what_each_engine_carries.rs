use amethystate::Store;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use serde::{Deserialize, Serialize};

mod common;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
enum Mode {
    Off,
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Refused,
    Kept,
    Changed(String),
}

fn attempt_where<T>(store: &Store, value: T, same: fn(&T, &T) -> bool) -> Outcome
where
    T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
{
    if store.set(["m", "v"], &value).is_err() {
        return Outcome::Refused;
    }
    match store.get::<T>(["m", "v"]) {
        Ok(Some(back)) if same(&back, &value) => Outcome::Kept,
        other => Outcome::Changed(format!("{other:?}")),
    }
}

fn attempt<T>(store: &Store, value: T) -> Outcome
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    attempt_where(store, value, |a, b| a == b)
}

struct Shape {
    name: &'static str,
    try_it: fn(&Store) -> Outcome,
    carried_by: fn(Backend) -> bool,
}

const SHAPES: &[Shape] = &[
    Shape {
        name: "an ordinary integer",
        try_it: |s| attempt(s, 42u32),
        carried_by: |_| true,
    },
    Shape {
        name: "a non-finite float",
        try_it: |s| attempt_where(s, f64::NAN, |a, b| a.is_nan() == b.is_nan()),
        carried_by: Backend::holds_non_finite_floats,
    },
    Shape {
        name: "an enum",
        try_it: |s| attempt(s, Mode::Off),
        carried_by: Backend::holds_enums,
    },
    Shape {
        name: "a Some holding nothing",
        try_it: |s| attempt(s, Some(None::<u32>)),
        carried_by: Backend::keeps_a_nested_option,
    },
    Shape {
        name: "an integer past i64",
        try_it: |s| attempt(s, u64::MAX),
        carried_by: Backend::holds_an_integer_past_i64,
    },
];

fn open(backend: Backend, promising: &[Backend], at: &TempPath) -> Store {
    let promised: Vec<Backend> = promising.to_vec();
    StoreBuilder::new(at.path())
        .backend(backend)
        .limits(move |l| l.portable_across(promised))
        .build()
        .unwrap()
}

/// The table every rule is folded over, checked against the engine itself
/// rather than taken on trust.
#[test]
fn what_an_engine_says_it_carries_is_what_it_carries() {
    let mut wrong = Vec::new();

    for backend in common::enabled_backends() {
        let path = TempPath::new("carries");
        let store = open(backend, &[], &path);

        for shape in SHAPES {
            let got = (shape.try_it)(&store);
            let claimed = (shape.carried_by)(backend);

            let agrees = match (&got, claimed) {
                (Outcome::Kept, true) => true,
                (Outcome::Refused, false) => true,
                _ => false,
            };

            if !agrees {
                wrong.push(format!(
                    "{}: {} claims {claimed} and answered {got:?}",
                    common::engine_name(backend),
                    shape.name
                ));
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A promise binds every engine named, so the answer is the table folded with
/// `and` - and a conjunction is settled by its members and the whole fold, so
/// nothing between them can disagree.
#[test]
fn a_promise_refuses_what_any_engine_it_named_cannot_carry() {
    let engines = common::enabled_backends();

    let mut promise_sets: Vec<Vec<Backend>> = vec![Vec::new()];
    for e in &engines {
        promise_sets.push(vec![*e]);
    }
    promise_sets.push(engines.clone());

    let mut wrong = Vec::new();

    for backend in &engines {
        for promised in &promise_sets {
            let path = TempPath::new("promise");
            let store = open(*backend, promised, &path);

            for shape in SHAPES {
                let expected = (shape.carried_by)(*backend)
                    && promised.iter().all(|e| (shape.carried_by)(*e));

                let got = (shape.try_it)(&store);

                let agrees = match (&got, expected) {
                    (Outcome::Kept, true) => true,
                    (Outcome::Refused, false) => true,
                    _ => false,
                };

                if !agrees {
                    let names: Vec<&str> =
                        promised.iter().map(|e| common::engine_name(*e)).collect();
                    wrong.push(format!(
                        "{} promising [{}]: {} should be {} and answered {got:?}",
                        common::engine_name(*backend),
                        names.join(", "),
                        shape.name,
                        if expected { "kept" } else { "refused" }
                    ));
                }
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The control: a store that refuses everything would pass the two above.
#[test]
fn an_ordinary_value_is_carried_by_every_engine_under_every_promise() {
    let engines = common::enabled_backends();

    for backend in &engines {
        let path = TempPath::new("control");
        let store = open(*backend, &engines, &path);

        assert_eq!(
            attempt(&store, 42u32),
            Outcome::Kept,
            "{} promising everything",
            common::engine_name(*backend)
        );
    }
}
