use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;

/// Values a migration step needs that are not in the store.
///
/// A step is a plain `fn(&mut MigrationContext)` collected at link time, so it
/// captures nothing: anything it needs from the application - a lookup table,
/// a client, the old config it is porting away from - has nowhere to arrive
/// except a global. This is that argument, handed over at
/// [`StoreBuilder::provide`](crate::StoreBuilder::provide) and read back by
/// type.
///
/// Keyed by [`TypeId`], so one value of each type. A step wanting two of the
/// same thing wraps them in a type that says which is which, which is also
/// what makes the call site legible.
///
/// Nothing here is `Send` or `Sync`, and deliberately. A GUI has plenty worth
/// handing a migration that is neither - an `Rc`, a `RefCell`, a handle its
/// toolkit refuses to move - and a value put here is only ever borrowed by
/// the step and dropped with the set, so a bound demanding otherwise would
/// rule those out to buy nothing.
///
/// The safety of that is the compiler's, not an assumption about where
/// migrations run. `MigrationContext` and `MigrationSet` are both public, so
/// a caller can drive a migration themselves rather than through
/// `StoreBuilder`, and "it happens on the thread that opened the store" is a
/// fact about today's engine rather than something to hang a bound on. What
/// holds regardless is that a `Provided` cannot reach another thread at all.
///
/// The cost, plainly: `Box<dyn Any>` erases auto traits instead of
/// propagating them, so `StoreBuilder` stops being `Send` once anything is
/// provided - even something that was `Send` itself.
#[derive(Default)]
pub struct Provided {
    values: HashMap<TypeId, Held>,
}

/// The value, and the name of its type - which `Any` erases and `TypeId`
/// cannot give back, so it is kept here rather than lost. A step asking for
/// something absent is an error path, and an error that can list what *is*
/// here is worth the one `&'static str`.
struct Held {
    type_name: &'static str,
    value: Box<dyn Any>,
}

impl Provided {
    /// Hands `value` to every migration step. Replaces one of the same type.
    pub fn insert<T: Any>(&mut self, value: T) {
        self.values.insert(
            TypeId::of::<T>(),
            Held {
                type_name: type_name::<T>(),
                value: Box::new(value),
            },
        );
    }

    /// Borrows what was provided for `T`, or `None` if nothing was.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|held| held.value.downcast_ref::<T>())
    }

    /// Every type provided, for a report that has to say what was on offer.
    pub(crate) fn type_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> =
            self.values.values().map(|held| held.type_name).collect();
        names.sort_unstable();
        names
    }
}
