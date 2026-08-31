use crate::store::facts::{Key, Prefix, Refused};
use crate::store::{StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::Report;
use std::any::{Any, TypeId, type_name};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// A rule a declared value has to pass on its way in from the store.
///
/// Written as a bare `fn` in `#[amestate(check = ..)]`, so it captures
/// nothing; what it needs from the application arrives through
/// [`CheckContext`], which [`StoreBuilder::context`](crate::StoreBuilder::context)
/// fills.
pub type Check<TValue> = fn(&TValue, &CheckContext) -> Result<(), Invalid>;

/// Values the application handed the store for its declared checks.
///
/// A check runs whenever a value arrives - while the struct is being built,
/// and again for every edit the file watcher brings in - so what it reads has
/// to be usable from whichever thread noticed the change. That is the whole of
/// the `Send + Sync` bound, and the whole of the difference from
/// [`StoreBuilder::provide`](crate::StoreBuilder::provide), which hands a value
/// to a migration step that runs once, inside `build`, on the thread that
/// called it.
///
/// Keyed by [`TypeId`], so one value of each type. Two of the same thing want
/// a type that says which is which, which is also what makes the call site
/// legible.
#[derive(Default)]
pub struct CheckContext {
    values: HashMap<TypeId, Held>,
}

struct Held {
    type_name: &'static str,
    value: Arc<dyn Any + Send + Sync>,
}

impl CheckContext {
    pub(crate) fn insert<T: Any + Send + Sync>(&mut self, value: T) {
        self.values.insert(
            TypeId::of::<T>(),
            Held {
                type_name: type_name::<T>(),
                value: Arc::new(value),
            },
        );
    }

    /// Borrows what the application gave for `T`, or `None` if it gave none.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|held| held.value.downcast_ref::<T>())
    }

    /// Borrows what the application gave for `T`, refusing the value if it
    /// gave none.
    ///
    /// A check that cannot reach its world cannot say the value is good, so
    /// the missing input travels the same way the verdict does, and the
    /// message lists what was on offer.
    pub fn require<T: Any + Send + Sync>(&self) -> Result<&T, Invalid> {
        match self.get::<T>() {
            Some(value) => Ok(value),
            None => Err(Invalid::new(format!(
                "no value provided for {}, so the check could not run; {}. \
                 StoreBuilder::context hands a value to every declared check",
                type_name::<T>(),
                self.on_offer()
            ))),
        }
    }

    fn on_offer(&self) -> String {
        let mut names: Vec<&'static str> = self.values.values().map(|held| held.type_name).collect();
        names.sort_unstable();

        if names.is_empty() {
            "nothing was given".to_string()
        } else {
            format!("given: {}", names.join(", "))
        }
    }
}

impl fmt::Debug for CheckContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckContext")
            .field("types", &self.on_offer())
            .finish()
    }
}

/// A check's verdict against a value, and why.
///
/// The reason is what [`Field::try_get`](crate::Field::try_get) reports and
/// what a refused open carries, so it is written for whoever has to fix the
/// file, and says what about the value was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    reason: Cow<'static, str>,
    at: &'static [&'static str],
}

impl Invalid {
    pub fn new(reason: impl Into<Cow<'static, str>>) -> Self {
        Self {
            reason: reason.into(),
            at: &[],
        }
    }

    /// Names the fields a struct's check is about, by their stored names.
    ///
    /// Only those fields report the refusal, so asking a field the invariant
    /// never mentioned still answers what it holds. A verdict that names none
    /// is about all of them.
    pub fn at(mut self, fields: &'static [&'static str]) -> Self {
        self.at = fields;
        self
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The fields named by [`Invalid::at`], or `None` for all of them.
    pub fn fields(&self) -> Option<&'static [&'static str]> {
        if self.at.is_empty() {
            None
        } else {
            Some(self.at)
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl From<&'static str> for Invalid {
    fn from(reason: &'static str) -> Self {
        Self::new(reason)
    }
}

impl From<String> for Invalid {
    fn from(reason: String) -> Self {
        Self::new(reason)
    }
}

/// The report a refused value fails an open with.
pub fn refused(path: &StorePath, invalid: &Invalid) -> Report<StorageError> {
    Report::new(StorageError::Read)
        .attach(Key(path.clone()))
        .attach(Refused(invalid.reason().to_string()))
}

/// The same, for a check declared on a struct, which is about the whole
/// prefix rather than one key.
pub fn refused_under(prefix: &StorePath, invalid: &Invalid) -> Report<StorageError> {
    Report::new(StorageError::Read)
        .attach(Prefix(prefix.clone()))
        .attach(Refused(invalid.reason().to_string()))
}

/// What a refused value does on the path that loads a plain struct, where
/// there is no field to hold the complaint.
///
/// [`OnUnreadable::Refuse`](crate::store::OnUnreadable::Refuse) fails the load.
/// [`OnUnreadable::UseDefault`](crate::store::OnUnreadable::UseDefault) takes
/// the declared default, and the log is the only place it is said - a loaded
/// struct is plain data with no `try_get` to ask.
pub fn refused_or_default<TValue>(
    path: &StorePath,
    invalid: Invalid,
    policy: crate::store::OnUnreadable,
    default: TValue,
) -> StorageResult<TValue> {
    match policy {
        crate::store::OnUnreadable::Refuse => Err(refused(path, &invalid)),
        crate::store::OnUnreadable::UseDefault => {
            tracing::error!(
                target: "amethystate",
                path = %path,
                reason = %invalid,
                "a declared check refused the stored value, so the field was loaded on its default"
            );
            Ok(default)
        }
    }
}
