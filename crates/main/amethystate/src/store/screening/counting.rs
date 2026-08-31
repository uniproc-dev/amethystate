//! A serializer that counts what goes through it and hands everything on.
//!
//! The count runs inside the codec's own pass, so one pass does both jobs and
//! `is_human_readable` is the codec's own answer - the question is forwarded to
//! it. A value whose `Serialize` branches on that answer, as `uuid` does when
//! it writes a string to json and sixteen bytes to msgpack, therefore shows the
//! count the shape that reaches the file.
//!
//! `serialize_some` and the compound methods take the nested value and hand it
//! to *their* inner serializer, so the wrapper has to reach every level through
//! the value: each nested value is paired with the budget and re-wraps whatever
//! serializer it is given. This is the shape `serde_stacker` uses to grow a
//! stack at every recursion point, counting where that one grows.

use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use std::cell::Cell;

/// What one serialization went past, and how deep it may still go.
///
/// Depth is the part with a limit of its own, because a pass that goes too
/// deep is stopped where it stands rather than read afterwards. The rest is
/// recorded and left for [`Screening`](super::Screening) to judge, since
/// whether a `NaN` or an enum is a refusal depends on the store rather than
/// the value.
///
/// Shared by every level of one pass: serde hands the serializer over by value
/// at each level, so the running count cannot live in it.
pub struct Noticed {
    depth: Cell<usize>,
    deepest: Cell<usize>,
    overflowed: Cell<bool>,
    non_finite: Cell<bool>,
    enum_variant: Cell<bool>,
    inside_a_some: Cell<bool>,
    collapsed_option: Cell<bool>,
    past_i64: Cell<bool>,
    limit: usize,
}

impl Noticed {
    /// A pass allowed `limit` levels.
    pub fn new(limit: usize) -> Self {
        Self {
            depth: Cell::new(0),
            deepest: Cell::new(0),
            overflowed: Cell::new(false),
            non_finite: Cell::new(false),
            enum_variant: Cell::new(false),
            inside_a_some: Cell::new(false),
            collapsed_option: Cell::new(false),
            past_i64: Cell::new(false),
            limit,
        }
    }

    /// A budget nothing can exhaust, for the store's own writes.
    ///
    /// The schema snapshot, the migration log and the initialization markers
    /// are shapes this crate declares itself, and a schema deep enough to
    /// matter would have had its data refused first. Spelled out so that a
    /// write with no limit reads as a decision someone made.
    pub fn unlimited() -> Self {
        Self::new(usize::MAX)
    }

    /// `serializer`, counted.
    pub fn wrap<S>(&self, serializer: S) -> Counting<'_, S> {
        Counting {
            inner: serializer,
            depth: self,
        }
    }

    /// `value`, counted by whatever serializer it is eventually handed to.
    ///
    /// The way in for a codec whose entry point takes the value and builds its
    /// own serializer inside, where [`Noticed::wrap`] has nothing to hold. The
    /// budget travels with the value, so it works whatever the codec does with
    /// it, and `is_human_readable` stays the codec's own answer because the
    /// question reaches it through the wrapper.
    pub fn count<'v, T>(&self, value: &'v T) -> Counted<'v, '_, T>
    where
        T: Serialize + ?Sized,
    {
        Counted { value, depth: self }
    }

    /// Whether the pass stopped because the value went past the limit.
    ///
    /// The refusal reaches the caller as the codec's own error type, since that
    /// is all a `Serializer` may return, so it cannot be recognised by its
    /// type. This says whether that error was ours.
    pub fn overflowed(&self) -> bool {
        self.overflowed.get()
    }

    /// Whether a `NaN` or an infinity went past during the pass.
    ///
    /// Noticed here because the pass is already happening; whether it is a
    /// refusal is the budget's question, since it depends on which codecs the
    /// store promised to stay readable on.
    ///
    /// Read after the pass, not during: a codec that writes `null` for such a
    /// value succeeds, so there is no error to carry the news.
    pub fn saw_a_non_finite_float(&self) -> bool {
        self.non_finite.get()
    }

    /// Whether an enum variant of any shape went past during the pass.
    ///
    /// Noticed here for the same reason and read the same way: whether it is a
    /// refusal belongs to the budget, and the codec that cannot hold one says
    /// nothing at the write.
    pub fn saw_an_enum(&self) -> bool {
        self.enum_variant.get()
    }

    /// Whether a `None` went past while it was the whole of what a `Some` held.
    ///
    /// `Some(None)` and `None` are one value to any format that writes an
    /// `Option` as a bare null, because the outer `Some` has nothing of its own
    /// to write. Only the value directly inside the `Some` counts: `Some` of a
    /// list holding a `None` is two levels apart and comes back whole.
    pub fn saw_a_collapsing_option(&self) -> bool {
        self.collapsed_option.get()
    }

    /// Whether an integer went past that does not fit in an `i64`.
    ///
    /// The only size TOML has, so a store on any other engine that promised to
    /// stay readable on toml has to answer for it too.
    pub fn saw_an_integer_past_i64(&self) -> bool {
        self.past_i64.get()
    }

    /// Called on the way into every method that is not `serialize_some`, so
    /// that what `serialize_none` sees is a `Some` it sits directly inside.
    fn left_the_some(&self) {
        self.inside_a_some.set(false);
    }

    /// The deepest level reached, once a pass has finished.
    pub fn deepest(&self) -> usize {
        self.deepest.get()
    }

    fn enter<E: ser::Error>(&self) -> Result<(), E> {
        self.left_the_some();

        let now = self.depth.get() + 1;
        if now > self.limit {
            self.overflowed.set(true);
            return Err(E::custom(format_args!(
                "the value nests deeper than {} levels",
                self.limit
            )));
        }
        self.depth.set(now);
        if now > self.deepest.get() {
            self.deepest.set(now);
        }
        Ok(())
    }

    fn leave(&self) {
        self.depth.set(self.depth.get().saturating_sub(1));
    }
}

/// A serializer that counts levels, notes what the store may refuse, and
/// forwards everything else.
pub struct Counting<'a, S> {
    inner: S,
    depth: &'a Noticed,
}

/// A value carrying the budget with it.
///
/// The inner serializer reaches for a nested value on its own - through
/// `serialize_some`, `serialize_element`, `serialize_field` - and hands it a
/// serializer of its own making. Pairing the value with the budget is what puts
/// the counter back in the way at every one of those levels.
pub struct Counted<'v, 'd, T: ?Sized> {
    value: &'v T,
    depth: &'d Noticed,
}

impl<T> Serialize for Counted<'_, '_, T>
where
    T: Serialize + ?Sized,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(Counting {
            inner: serializer,
            depth: self.depth,
        })
    }
}

/// Methods that open nothing: the value goes straight through.
macro_rules! forward {
    ($($method:ident($($arg:ident: $ty:ty),*);)+) => {
        $(fn $method(self $(, $arg: $ty)*) -> Result<S::Ok, S::Error> {
            self.depth.left_the_some();
            self.inner.$method($($arg),*)
        })+
    };
}

impl<'a, S: Serializer> Serializer for Counting<'a, S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = Counting<'a, S::SerializeSeq>;
    type SerializeTuple = Counting<'a, S::SerializeTuple>;
    type SerializeTupleStruct = Counting<'a, S::SerializeTupleStruct>;
    type SerializeTupleVariant = Counting<'a, S::SerializeTupleVariant>;
    type SerializeMap = Counting<'a, S::SerializeMap>;
    type SerializeStruct = Counting<'a, S::SerializeStruct>;
    type SerializeStructVariant = Counting<'a, S::SerializeStructVariant>;

    forward! {
        serialize_bool(v: bool);
        serialize_i8(v: i8);
        serialize_i16(v: i16);
        serialize_i32(v: i32);
        serialize_i64(v: i64);
        serialize_u8(v: u8);
        serialize_u16(v: u16);
        serialize_u32(v: u32);
        serialize_char(v: char);
        serialize_str(v: &str);
        serialize_bytes(v: &[u8]);
        serialize_unit();
        serialize_unit_struct(name: &'static str);
    }

    /// Integers that may not fit where they are going.
    ///
    /// TOML has one integer type and it is `i64`, so a `u64` past its top - or
    /// an `i128` past either end - is a value it cannot write. The three
    /// smaller unsigned types and `i64` itself always fit and go through the
    /// forwarding above.
    fn serialize_u64(self, v: u64) -> Result<S::Ok, S::Error> {
        self.depth.left_the_some();
        if v > i64::MAX as u64 {
            self.depth.past_i64.set(true);
        }
        self.inner.serialize_u64(v)
    }

    fn serialize_u128(self, v: u128) -> Result<S::Ok, S::Error> {
        self.depth.left_the_some();
        if v > i64::MAX as u128 {
            self.depth.past_i64.set(true);
        }
        self.inner.serialize_u128(v)
    }

    fn serialize_i128(self, v: i128) -> Result<S::Ok, S::Error> {
        self.depth.left_the_some();
        if v > i64::MAX as i128 || v < i64::MIN as i128 {
            self.depth.past_i64.set(true);
        }
        self.inner.serialize_i128(v)
    }

    /// The one place the nesting shows, and only for the pass that is
    /// happening: a format writing an `Option` as a bare null has already
    /// spent the outer `Some` by the time it gets here.
    fn serialize_none(self) -> Result<S::Ok, S::Error> {
        if self.depth.inside_a_some.get() {
            self.depth.collapsed_option.set(true);
        }
        self.depth.left_the_some();
        self.inner.serialize_none()
    }

    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> Result<S::Ok, S::Error> {
        self.depth.left_the_some();
        self.depth.enum_variant.set(true);
        self.inner.serialize_unit_variant(name, index, variant)
    }

    /// Forwarded like the rest, and noted on the way past.
    ///
    /// A `NaN` or an infinity is written by every codec here, and read back by
    /// three of them; the two that carry JSON write `null` and then fail to
    /// read it. Noting it costs one comparison on a pass that is happening
    /// anyway.
    fn serialize_f32(self, v: f32) -> Result<S::Ok, S::Error> {
        self.depth.left_the_some();
        if !v.is_finite() {
            self.depth.non_finite.set(true);
        }
        self.inner.serialize_f32(v)
    }

    fn serialize_f64(self, v: f64) -> Result<S::Ok, S::Error> {
        self.depth.left_the_some();
        if !v.is_finite() {
            self.depth.non_finite.set(true);
        }
        self.inner.serialize_f64(v)
    }

    /// `Some` is not a level: no format spends nesting on it, and counting it
    /// would make `Option<T>` measure deeper than the `T` inside.
    ///
    /// Which is the same reason it is the one method that arms rather than
    /// clears the flag: having spent nothing, it leaves a `None` directly
    /// inside it with nothing to tell it apart from a `None` on its own.
    fn serialize_some<T>(self, value: &T) -> Result<S::Ok, S::Error>
    where
        T: Serialize + ?Sized,
    {
        let Counting { inner, depth } = self;
        depth.inside_a_some.set(true);
        let out = inner.serialize_some(&Counted { value, depth });
        depth.left_the_some();
        out
    }

    fn serialize_newtype_struct<T>(self, name: &'static str, value: &T) -> Result<S::Ok, S::Error>
    where
        T: Serialize + ?Sized,
    {
        let Counting { inner, depth } = self;
        depth.left_the_some();
        inner.serialize_newtype_struct(name, &Counted { value, depth })
    }

    /// A newtype variant is a level: every format spells it as a wrapper around
    /// the value, whether that is `{"V": x}` or `V(x)`.
    fn serialize_newtype_variant<T>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<S::Ok, S::Error>
    where
        T: Serialize + ?Sized,
    {
        let Counting { inner, depth } = self;
        depth.left_the_some();
        depth.enum_variant.set(true);
        depth.enter()?;
        let out = inner.serialize_newtype_variant(name, index, variant, &Counted { value, depth });
        depth.leave();
        out
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, S::Error> {
        self.depth.enter()?;
        Ok(Counting {
            inner: self.inner.serialize_seq(len)?,
            depth: self.depth,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, S::Error> {
        self.depth.enter()?;
        Ok(Counting {
            inner: self.inner.serialize_tuple(len)?,
            depth: self.depth,
        })
    }

    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, S::Error> {
        self.depth.enter()?;
        Ok(Counting {
            inner: self.inner.serialize_tuple_struct(name, len)?,
            depth: self.depth,
        })
    }

    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, S::Error> {
        self.depth.enum_variant.set(true);
        self.depth.enter()?;
        Ok(Counting {
            inner: self
                .inner
                .serialize_tuple_variant(name, index, variant, len)?,
            depth: self.depth,
        })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, S::Error> {
        self.depth.enter()?;
        Ok(Counting {
            inner: self.inner.serialize_map(len)?,
            depth: self.depth,
        })
    }

    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, S::Error> {
        self.depth.enter()?;
        Ok(Counting {
            inner: self.inner.serialize_struct(name, len)?,
            depth: self.depth,
        })
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, S::Error> {
        self.depth.enum_variant.set(true);
        self.depth.enter()?;
        Ok(Counting {
            inner: self
                .inner
                .serialize_struct_variant(name, index, variant, len)?,
            depth: self.depth,
        })
    }

    /// The codec's own answer, which is the whole reason the count runs inside
    /// its pass: a `Serialize` that branches on this must be shown the shape
    /// that is going to be written.
    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }

    fn collect_str<T>(self, value: &T) -> Result<S::Ok, S::Error>
    where
        T: std::fmt::Display + ?Sized,
    {
        self.inner.collect_str(value)
    }
}

/// The compound halves, which differ only in the name of the method that takes
/// a value and whether it takes a key first.
macro_rules! compound {
    ($($trait_name:ident { $($method:ident($($arg:ident: $ty:ty),*)),+ $(,)? })+) => {
        $(impl<S: $trait_name> $trait_name for Counting<'_, S> {
            type Ok = S::Ok;
            type Error = S::Error;

            $(fn $method<T>(&mut self $(, $arg: $ty)*, value: &T) -> Result<(), S::Error>
            where
                T: Serialize + ?Sized,
            {
                let nested = Counted { value, depth: self.depth };
                self.inner.$method($($arg,)* &nested)
            })+

            fn end(self) -> Result<S::Ok, S::Error> {
                self.depth.leave();
                self.inner.end()
            }
        })+
    };
}

compound! {
    SerializeSeq { serialize_element() }
    SerializeTuple { serialize_element() }
    SerializeTupleStruct { serialize_field() }
    SerializeTupleVariant { serialize_field() }
    SerializeStruct { serialize_field(key: &'static str) }
    SerializeStructVariant { serialize_field(key: &'static str) }
}

impl<S: SerializeMap> SerializeMap for Counting<'_, S> {
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), S::Error>
    where
        T: Serialize + ?Sized,
    {
        let nested = Counted {
            value: key,
            depth: self.depth,
        };
        self.inner.serialize_key(&nested)
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), S::Error>
    where
        T: Serialize + ?Sized,
    {
        let nested = Counted {
            value,
            depth: self.depth,
        };
        self.inner.serialize_value(&nested)
    }

    fn end(self) -> Result<S::Ok, S::Error> {
        self.depth.leave();
        self.inner.end()
    }
}
