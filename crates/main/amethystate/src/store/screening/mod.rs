//! What a value has to be for this store to hand it back, decided while the
//! codec writes it.
//!
//! Three things are caught here, and they share a shape: the codec takes the
//! value, reports success, and the next read cannot make sense of what it
//! wrote. No error anywhere and the file is gone, which is the worst form a
//! defect has.
//!
//! Depth is the first. Every codec reads less deeply than it writes -
//! `serde_json` stops at 128 on the way in and has no limit on the way out,
//! `ron` stops at 64, and `rmp_serde` has no limit at all, so the stack runs
//! out around three thousand and kills the process on every later start
//! because the value is already committed. A non-finite float is the second:
//! JSON has no spelling for one, so `null` is written and fails to decode. An
//! enum is the third, on ron, whose document type has no variant to hold the
//! name.
//!
//! All three have to be learned from the write itself. By the time a value
//! reaches a store it is a `&dyn erased_serde::Serialize`, and building it out
//! to inspect it is the dangerous act - on redb it is what overflows the
//! stack. Serde is a push protocol and the store is on the receiving end, so
//! [`Counting`] watches what goes past during the codec's own pass and
//! [`Noticed`] holds what it saw. [`Screening`] decides which of it is a
//! refusal, because only it knows what the store promised.

mod counting;

pub use counting::{Counted, Counting, Noticed};

use crate::store::builder::Backend;
use crate::store::config::WriteLimits;
use crate::store::facts::Key;
use crate::store::{CodecFormat, StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::Report;

/// What one store will carry, worked out once when it opens.
///
/// Each answer is the running codec's, narrowed by whatever the store promised
/// to stay readable on. `key_depth` is the exception: the store's own cap on
/// paths, which is a setting rather than a fact about a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screening {
    pub ceiling: usize,
    pub key_depth: Option<usize>,

    /// Whether a `NaN` or an infinity survives on the running engine and on
    /// every engine this store promised to stay readable on.
    pub non_finite_floats: bool,

    /// The same for an enum of any shape.
    pub enums: bool,

    /// The same for `Some(None)`, which only ron tells apart from `None`.
    pub nested_options: bool,

    /// The same for an integer past `i64`, which toml has no room for.
    pub wide_integers: bool,
}

impl Screening {
    /// What a store running on `engine` under `limits` will carry.
    pub fn resolve(limits: &WriteLimits, engine: Backend) -> Self {
        Self {
            ceiling: limits.ceiling(engine),
            key_depth: limits.key_depth,
            non_finite_floats: limits.holds_non_finite_floats(engine),
            enums: limits.holds_enums(engine),
            nested_options: limits.keeps_a_nested_option(engine),
            wide_integers: limits.holds_an_integer_past_i64(engine),
        }
    }

    /// The same for an engine known only by the codec it runs, which is how a
    /// text store knows itself - it is generic over the document, not over the
    /// backend that chose it.
    pub fn for_codec(limits: &WriteLimits, codec: CodecFormat) -> Self {
        let engine = match codec {
            #[cfg(feature = "redb")]
            CodecFormat::MessagePack => Backend::Redb,
            #[cfg(feature = "json")]
            CodecFormat::Json => Backend::Json,
            #[cfg(feature = "sqlite")]
            CodecFormat::SonicJson => Backend::Sqlite,
            #[cfg(feature = "toml")]
            CodecFormat::Toml => Backend::Toml,
            #[cfg(feature = "ron")]
            CodecFormat::Ron => Backend::Ron,
            #[cfg(test)]
            CodecFormat::Default => {
                return Self {
                    ceiling: usize::MAX,
                    key_depth: limits.key_depth,
                    non_finite_floats: true,
                    enums: true,
                    nested_options: true,
                    wide_integers: true,
                };
            }
        };
        Self::resolve(limits, engine)
    }

    /// Whether a path is within the store's own cap on how deep a key may go.
    pub fn check_path(&self, path: &StorePath) -> StorageResult<()> {
        let levels = path.segments().count();

        if let Some(cap) = self.key_depth
            && levels > cap
        {
            return Err(Report::new(StorageError::Depth)
                .attach(Key(path.clone()))
                .attach(format!("levels: {levels}, and the limit is {cap}"))
                .attach("set by: limits(|l| l.key_depth(..))")
                .attach(format!(
                    "what is stored here spends the same budget - this store reads {} levels in all",
                    self.ceiling
                )));
        }

        Ok(())
    }

    /// What a value at `path` has left to spend, to be carried through the
    /// codec's own pass.
    ///
    /// The path is counted with the value because the budget is shared: on
    /// every text engine the path's levels become the document's, so a shallow
    /// value at a deep path is exactly as unreadable as a deep value at a
    /// shallow one. The flat engines keep the path as one key - `&str` on redb,
    /// `TEXT` on sqlite - and pay for it here anyway, which costs a handful of
    /// levels out of 512 and 127 and saves a second rule.
    pub fn for_value(&self, path: &StorePath) -> Noticed {
        Noticed::new(self.ceiling.saturating_sub(path.segments().count()))
    }

    /// Says what went wrong, once a codec's error turns out to have been the
    /// count's.
    ///
    /// A `Serializer` may only return its own error type, so the refusal
    /// reaches the caller wearing the codec's clothes and cannot be recognised
    /// by its type - [`Noticed::overflowed`] is how the caller asks whether it
    /// was this.
    pub fn too_deep(&self, path: &StorePath) -> Report<StorageError> {
        let levels = path.segments().count();
        let left = self.ceiling.saturating_sub(levels);

        Report::new(StorageError::Codec)
            .attach(Key(path.clone()))
            .attach(format!(
                "the path spends {levels} levels and the value goes past the {left} that are left"
            ))
            .attach(format!("this store reads at most {} levels", self.ceiling))
            .attach(
                "a value deeper than the reader accepts is written without complaint and \
                 cannot be read back",
            )
    }

    /// Whether a pass that has finished wrote something this store cannot read
    /// back, or cannot promise elsewhere.
    ///
    /// Asked after a *successful* write rather than after a failed one: a codec
    /// with no spelling for a `NaN` writes `null` and reports success, so there
    /// is no error to inspect. That is the whole reason the value has to be
    /// refused here - left alone it lands as `null`, the write says `Ok`, and
    /// the field goes on reporting the number it held before while the file
    /// holds nothing of the sort.
    pub fn refused(&self, seen: &Noticed, path: &StorePath) -> Option<Report<StorageError>> {
        if !self.non_finite_floats && seen.saw_a_non_finite_float() {
            return Some(
                Report::new(StorageError::Codec)
                    .attach(Key(path.clone()))
                    .attach("a NaN or an infinity, which this store cannot read back")
                    .attach(
                        "JSON has no spelling for either, so the codec writes `null` and \
                         decoding it as a float fails - on json, and on sqlite, which encodes \
                         with the same JSON",
                    ),
            );
        }

        if !self.wide_integers && seen.saw_an_integer_past_i64() {
            return Some(
                Report::new(StorageError::Codec)
                    .attach(Key(path.clone()))
                    .attach("an integer that does not fit in an `i64`")
                    .attach(
                        "TOML has one integer type and it is signed and 64 bits wide, so a \
                         `u64` past its top has nowhere to go",
                    ),
            );
        }

        if !self.nested_options && seen.saw_a_collapsing_option() {
            return Some(
                Report::new(StorageError::Codec)
                    .attach(Key(path.clone()))
                    .attach("a `Some` holding nothing, which reads back as nothing at all")
                    .attach(
                        "the outer `Some` has nothing of its own to write, so `Some(None)` and \
                         `None` reach the file as one null - `c0` under msgpack, `null` under \
                         either JSON - and both come back `None`. ron is the one engine that \
                         spells the `Option` out and keeps them apart",
                    ),
            );
        }

        if !self.enums && seen.saw_an_enum() {
            return Some(
                Report::new(StorageError::Codec)
                    .attach(Key(path.clone()))
                    .attach("an enum, which this store cannot read back")
                    .attach(
                        "ron writes one as `On(3)` and parses it back into a `ron::value::Value`, \
                         which has no variant to put it in - the name is dropped there and the \
                         next read is handed a sequence. `Value` not supporting enums is \
                         listed in https://github.com/ron-rs/ron/issues/122",
                    ),
            );
        }

        None
    }
}
