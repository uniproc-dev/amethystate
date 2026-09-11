use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

/// A level's name as a path holds it.
///
/// Re-exported because it is part of what [`StorePath::try_push_shared`] takes
/// and what a document engine hands a scan: a caller naming that type should
/// not have to agree with this crate about a dependency to do it. Inline up to
/// 23 bytes, which is most level names, so holding one allocates nothing.
pub use smallvec::SmallVec;
pub use smol_str::SmolStr;

/// What divides one level from the next in a stored key.
///
/// Public because a backend that queries by comparison has to reason about it:
/// the upper bound of a subtree's range is this character's successor, and
/// spelling it out here is what makes every backend compute the same bound.
pub const SEPARATOR: char = '.';

pub(crate) const ESCAPE: char = '\\';

/// Where a value lives in the store, as the levels it is under.
///
/// A path is built from segments and only from segments, so a name is a name:
/// putting `"dark.mode"` in one addresses a single value with a dot in it, not
/// two levels.
///
/// Two forms answer the two kinds of engine, and a path holds whichever one it
/// arrived in. A flat engine keeps a key whole and addresses it by the joined
/// spelling; a document engine keeps a level at a time and addresses it by the
/// levels. The other form is built on the first reader that wants it and kept,
/// so a path costs one of the two, and a scan that never asks for the other
/// never pays for it.
///
/// Which is not a detail: both directions are taken per key of every scan.
/// Splitting a key allocates for the list of levels; spelling levels out walks
/// them for a separator and an escape and builds a string. Whichever a caller
/// does not read is the whole of that cost.
#[derive(Clone)]
pub struct StorePath {
    held: Held,
}

#[derive(Clone)]
enum Held {
    /// Written out where the code is compiled, so both forms are ready.
    Written {
        levels: &'static [&'static str],
        joined: &'static str,
    },

    /// Built from its levels, which is how a document engine reads one back.
    ///
    /// A level comes back borrowed. The spelling is not there until something
    /// asks - equality, order and hashing all answer from it, and so does any
    /// flat engine - and then it is kept, and every copy of the path made
    /// before or after shares it.
    ///
    /// Shared rather than held inline because a `StorePath` written into a
    /// `const` is borrowed for the whole program, and nothing borrowed that way
    /// may carry a cell. Behind the `Arc` the cell is reached through a pointer,
    /// which is what the macro's declared paths need it to be.
    Levels(Arc<Levels>),

    /// Read as one key, which is how a flat engine reads one back.
    ///
    /// The levels are not there until something walks them, and then they are
    /// walked out of the string and carried away by nobody. A level comes back
    /// as a `Cow` for that reason: one holding an escaped separator is not a
    /// run of the joined form and has to be assembled, and one without is
    /// borrowed from it.
    Joined { joined: SmolStr },

    /// The first `levels` of another path's, sharing the list they came from.
    ///
    /// What [`StorePath::parent`] is for. A path's ancestors are its own levels
    /// with the end taken off, so copying the list to shorten it is copying a
    /// thing to keep most of it - and the shortening is asked for in loops, one
    /// ancestor after another, where every copy is thrown away after a
    /// comparison. Here the list is not copied at all: the count says where to
    /// stop reading it.
    ///
    /// The spelling is the parent's, cut at the separator that ends the last
    /// level kept - a run of a string the `Arc` already owns, so there is
    /// nothing to store and no cell to carry.
    Prefix { of: Arc<Levels>, levels: usize },
}

/// The levels of a path, and the spelling once somebody has asked for it.
///
/// Both halves are [`SmolStr`], which holds a string of 23 bytes or fewer
/// inline and allocates nothing for it. That is most level names, and a good
/// share of whole paths; anything longer goes to the heap behind a count, which
/// is what it would have cost anyway. The level list is copied whole every time
/// a path grows one - `push`, `join`, `parent` and `strip_prefix` all go through
/// [`StorePath::own_levels`] - so the copies are where this is felt.
struct Levels {
    names: Box<[SmolStr]>,
    joined: OnceLock<SmolStr>,
}

/// Where the first `levels` of `joined` end: the byte the separator after them
/// sits at, or the whole length when there is no separator left to reach.
///
/// The escape is honoured, so a name holding one does not end a level early.
fn end_of_level(joined: &str, levels: usize) -> usize {
    if levels == 0 {
        return 0;
    }

    let mut seen = 0;
    let mut escaped = false;

    for (at, ch) in joined.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            ESCAPE => escaped = true,
            SEPARATOR => {
                seen += 1;
                if seen == levels {
                    return at;
                }
            }
            _ => {}
        }
    }

    joined.len()
}

/// How many levels a validated joined key holds, without building any of them.
fn count_levels(joined: &str) -> usize {
    if joined.is_empty() {
        return 0;
    }

    let mut levels = 1;
    let mut escaped = false;
    for ch in joined.chars() {
        match ch {
            _ if escaped => escaped = false,
            ESCAPE => escaped = true,
            SEPARATOR => levels += 1,
            _ => {}
        }
    }
    levels
}

/// One level of a validated joined key, borrowed unless it was escaped.
fn level_at(joined: &str, index: usize) -> Option<Cow<'_, str>> {
    let mut level = 0;
    let mut start = 0;
    let mut escaped = false;

    for (at, ch) in joined.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            ESCAPE => escaped = true,
            SEPARATOR => {
                if level == index {
                    return Some(unescape(&joined[start..at]));
                }
                level += 1;
                start = at + ch.len_utf8();
            }
            _ => {}
        }
    }

    (level == index && !joined.is_empty()).then(|| unescape(&joined[start..]))
}

impl StorePath {
    /// The path that is under nothing.
    pub const fn root() -> Self {
        Self {
            held: Held::Written {
                levels: &[],
                joined: "",
            },
        }
    }

    /// A path whose levels are known when the code is compiled.
    ///
    /// Both forms are handed over ready, so this allocates nothing: no level may
    /// be empty, and `joined` must be exactly what [`StorePath::as_str`] would
    /// produce for `segments`.
    ///
    /// Both are checked here rather than trusted. The check is a `const fn`, so
    /// a path written into a `const` - `StateScope::PATH`, which is where these
    /// come from - fails to compile when the halves disagree, whether a macro or
    /// a hand-written impl wrote them. A const panic carries no formatting, so
    /// it names the invariant and not the level; the `#[amethystate(prefix =
    /// ...)]` macro checks the same two things first and points at the
    /// attribute.
    ///
    /// # Panics
    ///
    /// If any level is empty, or if `joined` is not the joined form of
    /// `segments`. At compile time when the call is in a const context, which is
    /// the only place it is meant to be.
    pub const fn from_static(segments: &'static [&'static str], joined: &'static str) -> Self {
        check_static(segments, joined);

        Self {
            held: Held::Written {
                levels: segments,
                joined,
            },
        }
    }

    /// One level named `name`, whatever `name` contains - except nothing.
    ///
    /// # Panics
    ///
    /// If `name` is empty. Use [`StorePath::try_segment`] for a name that comes
    /// from data rather than from the source.
    #[track_caller]
    pub fn segment(name: impl AsRef<str>) -> Self {
        Self::from_segments([name])
    }

    /// [`StorePath::segment`] for a name that can turn out to be empty.
    pub fn try_segment(name: impl AsRef<str>) -> Result<Self, StorePathError> {
        Self::try_from_segments([name])
    }

    /// The whole of this path as one level, its spelling being the name.
    ///
    /// What a store with no levels is addressed by: `ui.theme` becomes one
    /// level called `ui.theme`, and a level of the original holding a `.` keeps
    /// the escape that tells it from a separator, so the two spell differently
    /// here as well.
    ///
    /// # Panics
    ///
    /// If this is the root, which spells nothing and so names no level.
    #[track_caller]
    pub fn as_one_level(&self) -> Self {
        Self::segment(PathRef::from(self).as_str())
    }

    /// A path out of the levels it is under, outermost first.
    ///
    /// # Panics
    ///
    /// If any level is empty, which is a path that cannot exist - see
    /// [`StorePathError::EmptySegment`]. Written-out levels are the source's to
    /// get right; levels that come from data go through
    /// [`StorePath::try_from_segments`] instead, and every call that builds a
    /// path out of a caller's strings - [`IntoStorePath`], and so `Store::get`,
    /// `Kv::set`, `ReactiveMap::insert` - already does.
    /// No levels at all is the root, and here that is a statement rather than
    /// an accident: an `as_root` struct's path is written out as no levels.
    /// [`StorePath::try_from_segments`] refuses it, because a list that came
    /// from data and turned out empty is not something anyone said.
    #[track_caller]
    pub fn from_segments<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        match Self::try_from_segments(segments) {
            Ok(path) => path,
            Err(StorePathError::EmptyPath) => Self::root(),
            Err(other) => panic!("{other}"),
        }
    }

    /// [`StorePath::from_segments`] for levels that can turn out to be empty.
    ///
    /// Which is where the two differ: a written-out empty list means the root,
    /// and a computed one that filtered down to nothing means nobody decided.
    /// The second is refused with [`StorePathError::EmptyPath`], because it
    /// would name the whole store, and a write through it would replace
    /// everything.
    pub fn try_from_segments<I, S>(segments: I) -> Result<Self, StorePathError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let segments = segments.into_iter();
        let mut collected: Vec<SmolStr> = Vec::with_capacity(segments.size_hint().0);

        for (at, segment) in segments.enumerate() {
            let segment = segment.as_ref();
            if segment.is_empty() {
                return Err(StorePathError::EmptySegment { at });
            }
            collected.push(SmolStr::new(segment));
        }

        if collected.is_empty() {
            return Err(StorePathError::EmptyPath);
        }

        Ok(Self::from_checked(collected))
    }

    fn from_checked(segments: Vec<SmolStr>) -> Self {
        Self {
            held: Held::Levels(Arc::new(Levels {
                names: segments.into_boxed_slice(),
                joined: OnceLock::new(),
            })),
        }
    }

    /// This path with one more level under it.
    ///
    /// # Panics
    ///
    /// If `name` is empty. Use [`StorePath::try_push`] for a name that comes
    /// from data.
    #[track_caller]
    pub fn push(&self, name: impl AsRef<str>) -> Self {
        self.try_push(name).expect("a path segment cannot be empty")
    }

    /// [`StorePath::push`] for a name that can turn out to be empty.
    pub fn try_push(&self, name: impl AsRef<str>) -> Result<Self, StorePathError> {
        self.try_push_shared(SmolStr::new(name.as_ref()))
    }

    /// [`StorePath::try_push`] taking a name the caller already holds shared.
    ///
    /// What a document engine has: a level's name is stored as one of these,
    /// and a scan puts a path together out of names it is holding anyway. The
    /// other form copies the name into a fresh one per key.
    pub fn try_push_shared(&self, name: SmolStr) -> Result<Self, StorePathError> {
        if name.is_empty() {
            return Err(StorePathError::EmptySegment { at: self.len() });
        }

        let mut segments = self.own_levels();
        segments.push(name);
        Ok(Self::from_checked(segments))
    }

    /// This path with `other`'s levels under it.
    pub fn join(&self, other: &StorePath) -> Self {
        let mut segments = self.own_levels();
        segments.extend(other.own_levels());
        Self::from_checked(segments)
    }

    /// The levels, outermost first.
    ///
    /// Borrowed where the level is a run of the joined form, which is every
    /// level whose name carries no separator to escape.
    pub fn segments(&self) -> impl ExactSizeIterator<Item = Level<'_>> + '_ {
        (0..self.len()).map(|i| self.segment_at(i).expect("counted"))
    }

    /// One level, or `None` past the end.
    pub fn segment_at(&self, index: usize) -> Option<Level<'_>> {
        match &self.held {
            Held::Written { levels, .. } => {
                levels.get(index).copied().map(Level::named)
            }
            Held::Levels(held) => held.names.get(index).map(|level| Level::named(level)),
            Held::Joined { joined } => level_at(joined, index).map(Level::from_cow),
            Held::Prefix { of, levels } => of
                .names
                .get(index)
                .filter(|_| index < *levels)
                .map(|level| Level::named(level)),
        }
    }

    /// The whole path as one string, with the separator escaped inside names.
    ///
    /// This is how a document's plane names a key, where the file has to hold
    /// something a person can read. A path that arrived as one pays nothing to
    /// read it back; a path built from levels is spelled here, once, and holds
    /// on to the spelling.
    ///
    /// Private on purpose. A path is its levels, and everything outside this
    /// module addresses one by them: the plane reaches the spelling through
    /// [`PathRef`], which is the type that says a joined key is meant, and
    /// anything that only wants to *show* a path uses [`Display`]. Handing the
    /// spelling out is how `&str` got back into signatures that had a path to
    /// hand.
    fn as_str(&self) -> &str {
        match &self.held {
            Held::Written { joined, .. } => joined,
            Held::Levels(held) => held.joined.get_or_init(|| join(&held.names)),
            Held::Joined { joined } => joined,

            // A run of the spelling the list it came from already has, so
            // asking for one spells the *parent* once and every ancestor of it
            // for nothing after that. Finding where to cut is a walk of the
            // string rather than a lookup, which is the price of not storing a
            // second spelling per ancestor.
            Held::Prefix { of, levels } => {
                let whole = of.joined.get_or_init(|| join(&of.names));
                &whole[..end_of_level(whole, *levels)]
            }
        }
    }

    /// The levels as their own list, for the callers that build a new path out
    /// of them.
    fn own_levels(&self) -> Vec<SmolStr> {
        match &self.held {
            Held::Written { levels, .. } => levels
                .iter()
                .map(|level| SmolStr::new_static(level))
                .collect(),
            Held::Levels(held) => held.names.to_vec(),
            Held::Joined { joined } => (0..count_levels(joined))
                .map(|at| SmolStr::new(&*level_at(joined, at).expect("counted")))
                .collect(),
            Held::Prefix { of, levels } => of.names[..*levels].to_vec(),
        }
    }

    /// Whether this path names the whole store rather than anything in it.
    ///
    /// Answered per state rather than through [`len`](Self::len), because
    /// counting the levels of a joined key walks the whole of it - and that is
    /// the state a flat engine's keys arrive in, asked once per key of every
    /// scan through `level_under`.
    pub fn is_root(&self) -> bool {
        match &self.held {
            Held::Joined { joined } => joined.is_empty(),
            Held::Written { levels, .. } => levels.is_empty(),
            Held::Levels(held) => held.names.is_empty(),
            Held::Prefix { levels, .. } => *levels == 0,
        }
    }

    /// How many levels the path has.
    ///
    /// There is deliberately no `is_empty` beside it. A path of no levels is
    /// the root - the one that names the whole store - and that is a thing
    /// rather than the absence of one, so [`is_root`](Self::is_root) is what
    /// asks. `is_empty` would be a second name for the same question, and the
    /// worse of the two.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        match &self.held {
            Held::Written { levels, .. } => levels.len(),
            Held::Levels(held) => held.names.len(),
            Held::Joined { joined } => count_levels(joined),
            Held::Prefix { levels, .. } => *levels,
        }
    }

    /// Whether every level of `prefix` starts this path.
    ///
    /// Compared level by level, so `ui` does not start `uix.width` - which
    /// comparing the joined strings would say it does.
    pub fn starts_with(&self, prefix: &StorePath) -> bool {
        prefix.len() <= self.len() && prefix.segments().zip(self.segments()).all(|(a, b)| a == b)
    }

    /// The levels below `prefix`, or `None` when `prefix` does not start this
    /// path.
    pub fn strip_prefix(&self, prefix: &StorePath) -> Option<StorePath> {
        self.starts_with(prefix)
            .then(|| StorePath::from_checked(self.own_levels()[prefix.len()..].to_vec()))
    }

    /// Whether this subtree and `other`'s hold any key in common.
    ///
    /// Two subtrees are nested or apart and never half over each other, so this
    /// is two containment tests and equality needs no arm of its own:
    /// [`starts_with`](Self::starts_with) admits the prefix itself.
    pub fn overlaps(&self, other: &StorePath) -> bool {
        self.starts_with(other) || other.starts_with(self)
    }

    /// The level list to hang a prefix off, made once where there is not one
    /// already.
    fn shared_levels(&self) -> Arc<Levels> {
        match &self.held {
            Held::Levels(held) => held.clone(),
            Held::Prefix { of, .. } => of.clone(),
            _ => Arc::new(Levels {
                names: self.own_levels().into_boxed_slice(),
                joined: OnceLock::new(),
            }),
        }
    }

    /// The path one level up, or `None` at the root.
    ///
    /// Shares the level list rather than copying it short: walking up is asked
    /// for one ancestor after another, and each answer used to be the whole
    /// list copied to drop the end of it.
    pub fn parent(&self) -> Option<StorePath> {
        let levels = self.len().checked_sub(1)?;

        Some(StorePath {
            held: Held::Prefix {
                of: self.shared_levels(),
                levels,
            },
        })
    }

    /// This path and every path it sits under, nearest first, the root last.
    ///
    /// The set a place that holds this one can be at: a path is held at itself
    /// or at one of its ancestors, and nowhere else. Walking it costs one level
    /// list, made once and shared by every step - where asking for a parent and
    /// then its parent used to copy the list per step, and throw each copy away
    /// after a comparison.
    pub fn upwards(&self) -> impl Iterator<Item = StorePath> + '_ {
        let of = self.shared_levels();

        (0..=self.len()).rev().map(move |levels| StorePath {
            held: Held::Prefix {
                of: of.clone(),
                levels,
            },
        })
    }

    /// The last level, or `None` at the root.
    pub fn name(&self) -> Option<Level<'_>> {
        self.segment_at(self.len().checked_sub(1)?)
    }

    /// Where this path sits under `prefix`, and the level it sits at.
    ///
    /// The method [`level_under`] would be if it had nothing to check. That
    /// one takes a key an engine read off a disk and can refuse it; a path has
    /// already been through that, so this only reads.
    ///
    /// Which is what lets the rest of the library ask the question without
    /// holding a key: an entry one level down and one two levels down are
    /// different answers, and telling them apart is a map's whole business.
    pub fn level_under(&self, prefix: &StorePath) -> Under<'_> {
        level_below(self.as_str(), prefix.as_str(), prefix.is_root())
    }

    /// The level that follows `prefix`, read straight off the joined form.
    ///
    /// The same answer as `starts_with(prefix)` followed by
    /// `segment_at(prefix.len())`, reached without splitting either path into
    /// levels. That matters where it is used: a scan hands back a path per
    /// stored key, and the caller wants one name from each, so splitting every
    /// key into a level per allocation is work thrown away as soon as it is
    /// done.
    ///
    /// Borrowed unless the name carries an escaped separator, which is the
    /// only case where the level is not a run of the joined string.
    pub fn name_under(&self, prefix: &StorePath) -> Option<Level<'_>> {
        match level_below(self.as_str(), prefix.as_str(), prefix.is_root()) {
            Under::Entry(name) | Under::Deeper(name) => Some(name),
            Under::Prefix | Under::Outside => None,
        }
    }

    /// The name `key` is stored under, below this path: the *last* level of
    /// what remains, where [`StorePath::name_under`] is the first.
    ///
    /// Taking a prefix off does not touch the end, so the last level of what
    /// remains is the last level of `key` - there is no remainder to build, and
    /// the name is borrowed out of `key` rather than copied out of it.
    pub fn entry_name<'k>(&self, key: &'k StorePath) -> Option<Level<'k>> {
        if !key.starts_with(self) || key.len() == self.len() {
            return None;
        }

        key.name()
    }

    /// Reads back what [`StorePath::as_str`] wrote.
    ///
    /// Only for data already on disk: a path in code is built from its
    /// segments, so nothing else needs to parse one. Fallible because a key
    /// this library did not write can hold a level with no name, and such a
    /// path is not one.
    pub fn parse_joined(joined: &str) -> Result<Self, StorePathError> {
        validate_joined(joined)?;

        Ok(Self {
            held: Held::Joined {
                joined: SmolStr::new(joined),
            },
        })
    }
}

/// A path that borrows its joined form instead of owning one.
///
/// What an engine has in hand when it reads a key out of its own page: the
/// bytes are there, spelled the way [`StorePath::as_str`] spells them, and
/// building a `StorePath` per key would allocate once for every entry on the
/// hot path of loading a map.
///
/// It carries the same checked-ness as an owned path and can be had no other
/// way: either [`PathRef::parse`] walked the key, or it borrows a path that
/// was walked already. So a caller holding one needs no opinion about how a
/// path is spelled, which is the whole point - the alternative is passing
/// `&str` and hoping every reader remembers what it is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathRef<'a> {
    joined: &'a str,
}

impl<'a> PathRef<'a> {
    /// Reads a key an engine holds, refusing one this library did not write.
    ///
    /// The same walk [`StorePath::parse_joined`] makes, without the
    /// allocation that follows it.
    pub fn parse(joined: &'a str) -> Result<Self, StorePathError> {
        validate_joined(joined)?;

        Ok(Self { joined })
    }

    /// The whole path as one string, which is what it was built from.
    pub fn as_str(&self) -> &'a str {
        self.joined
    }

    pub fn is_root(&self) -> bool {
        self.joined.is_empty()
    }

    /// Where this sits under `prefix`, and the level it sits at. See
    /// [`StorePath::level_under`], which answers the same question of an owned
    /// path.
    pub fn level_under(&self, prefix: &StorePath) -> Under<'a> {
        level_below(self.joined, prefix.as_str(), prefix.is_root())
    }

    /// The level directly under `prefix`, or `None` when `prefix` does not
    /// hold this path. See [`StorePath::name_under`].
    pub fn name_under(&self, prefix: &StorePath) -> Option<Level<'a>> {
        match self.level_under(prefix) {
            Under::Entry(name) | Under::Deeper(name) => Some(name),
            Under::Prefix | Under::Outside => None,
        }
    }

    /// An owned copy, for a caller that has to keep it past the borrow.
    ///
    /// This is where the allocation the type exists to avoid is finally paid,
    /// so it is paid by whoever actually keeps the path.
    pub fn to_path(&self) -> StorePath {
        StorePath::parse_joined(self.joined).expect("a borrowed path was checked when it was made")
    }
}

/// Borrowing a path costs nothing and checks nothing: it has been through the
/// check already.
impl<'a> From<&'a StorePath> for PathRef<'a> {
    fn from(path: &'a StorePath) -> Self {
        Self {
            joined: path.as_str(),
        }
    }
}

impl fmt::Debug for PathRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PathRef({:?})", self.joined)
    }
}

impl fmt::Display for PathRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.joined)
    }
}

impl PartialEq<StorePath> for PathRef<'_> {
    fn eq(&self, other: &StorePath) -> bool {
        self.joined == other.as_str()
    }
}

/// One level of a path: a name, as the store means a name.
///
/// Not a run of a joined key. A name holding a separator is still one name -
/// `dark.mode` is a level called `dark.mode`, not two - and the joined form
/// spells it with the separator doubled. Which is the whole reason this is a
/// type: the two are the same characters often enough that a `&str` between them
/// says nothing, and wrong often enough that it matters.
///
/// Borrowed where the level is a run of the joined form, which is every level
/// whose name carries nothing to escape, and held where it is not. A held name
/// is a [`SmolStr`], the same way the level list of a path holds one, so a name
/// of 23 bytes or fewer costs no allocation to keep.
#[derive(Clone)]
pub struct Level<'a>(Name<'a>);

#[derive(Clone)]
enum Name<'a> {
    Borrowed(&'a str),
    Held(SmolStr),
}

impl<'a> Level<'a> {
    /// A name a caller wrote or a document holds, taken as the name it is.
    ///
    /// The entry point from outside: a level read off a document's own map, or
    /// handed in by whoever is addressing the store. Nothing is checked here
    /// beyond what the type says - a name is any string that is not empty - and
    /// [`StorePath::try_segment`] is where emptiness is refused.
    pub fn named(name: &'a str) -> Self {
        Self(Name::Borrowed(name))
    }

    /// The name as characters, for handing to a document or an engine.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            Name::Borrowed(name) => name,
            Name::Held(name) => name,
        }
    }

    /// The name with a life of its own.
    pub fn into_owned(self) -> Level<'static> {
        Level(match self.0 {
            Name::Borrowed(name) => Name::Held(SmolStr::new(name)),
            Name::Held(name) => Name::Held(name),
        })
    }

    fn from_cow(name: Cow<'a, str>) -> Self {
        Self(match name {
            Cow::Borrowed(name) => Name::Borrowed(name),
            Cow::Owned(name) => Name::Held(SmolStr::new(name)),
        })
    }
}

impl Level<'static> {
    /// A name already held the way a level holds one, taken without copying it
    /// again.
    ///
    /// For a caller that has spelled the name itself - see
    /// [`SmolStrBuilder`](smol_str::SmolStrBuilder), which writes a short one
    /// into the inline form and never reaches the heap.
    pub fn held(name: SmolStr) -> Self {
        Self(Name::Held(name))
    }
}

/// By the name, whichever form holds it, so a level that was assembled answers
/// like the one that was borrowed.
impl PartialEq for Level<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Level<'_> {}

impl Ord for Level<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for Level<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Level<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

/// A level is its name, and equality, order and hashing all answer from it -
/// which is what lets a collection keyed by a level be looked up by one.
impl std::borrow::Borrow<str> for Level<'_> {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for Level<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Level({:?})", self.as_str())
    }
}

impl fmt::Display for Level<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A level is a name, and [`StorePath::from_segments`] takes names.
impl AsRef<str> for Level<'_> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for Level<'_> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Level<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// The name a document holds a child under.
///
/// Two different things that are the same characters. A tree's level is held
/// under its own name; the plane holds a whole path under the spelling of it,
/// because the plane's whole point is that a path is one key. Which of the two
/// a caller means is said here, by the constructor it reached for, instead of
/// being left to whoever reads the `&str` to work out - and it is the seam an
/// obfuscated key would be introduced along, because that is exactly the day
/// the two stop being the same characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored<'a>(&'a str);

impl<'a> Stored<'a> {
    /// A level of a tree, held under its own name.
    pub fn level(name: &'a Level<'_>) -> Self {
        Self(name.as_str())
    }

    /// A key of the plane, held under the whole path's spelling.
    pub fn whole(path: PathRef<'a>) -> Self {
        Self(path.as_str())
    }

    /// A name a document read out of its own map, taken as it stands.
    ///
    /// The entry point from the file: what is there is what is there, and
    /// nothing about it has been decided yet.
    pub fn read(name: &'a str) -> Self {
        Self(name)
    }

    /// The characters, for the format's own library to look up.
    pub fn as_str(&self) -> &'a str {
        self.0
    }
}

impl fmt::Display for Stored<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Where a key read back from a store sits relative to the prefix it was
/// scanned from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Under<'a> {
    /// The key is the prefix itself.
    Prefix,

    /// The key is exactly one level below, named here.
    Entry(Level<'a>),

    /// The key is more than one level below. The name is the level directly
    /// under the prefix; the rest of the key is inside it.
    Deeper(Level<'a>),

    /// The key is not under the prefix.
    Outside,
}

/// Where `key` sits under `prefix`, without building a path for it.
///
/// What a scan's caller usually wants from a key: a map keyed by the level
/// below its own path reads every key once and asks for one name from each, so
/// building a [`StorePath`] to answer that is a string and a walk thrown away
/// per entry.
///
/// [`Under::Entry`] and [`Under::Deeper`] are separate because a map owns the
/// level below it and nothing further: a key two levels down is somebody else's
/// and reading it as an entry hands back the wrong value under the wrong name.
///
/// The key is still checked, because a key this library did not write has to
/// be refused where it is read rather than believed.
pub fn level_under<'a>(key: &'a str, prefix: &StorePath) -> Result<Under<'a>, StorePathError> {
    validate_joined(key)?;
    Ok(level_below(key, prefix.as_str(), prefix.is_root()))
}

/// The level after `head` in `whole`, both joined.
fn level_below<'a>(whole: &'a str, head: &str, head_is_root: bool) -> Under<'a> {
    let rest = if head_is_root {
        whole
    } else if whole == head {
        return Under::Prefix;
    } else {
        match whole
            .strip_prefix(head)
            .and_then(|rest| rest.strip_prefix(SEPARATOR))
        {
            Some(rest) => rest,
            None => return Under::Outside,
        }
    };

    if rest.is_empty() {
        return Under::Prefix;
    }

    let mut escaped = false;
    for (at, ch) in rest.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            ESCAPE => escaped = true,
            SEPARATOR => return Under::Deeper(Level::from_cow(unescape(&rest[..at]))),
            _ => {}
        }
    }

    Under::Entry(Level::from_cow(unescape(rest)))
}

/// Whether a joined key is one this type could have written, without building
/// anything to find out.
///
/// The same walk the split does, counting levels instead of collecting them.
/// It runs eagerly where the split does not, because a key that will not parse
/// has to be refused where it is read rather than wherever someone first asks
/// for its levels.
fn validate_joined(joined: &str) -> Result<(), StorePathError> {
    let mut at_level = 0;
    let mut level_len = 0usize;
    let mut escaped = false;

    for ch in joined.chars() {
        match ch {
            _ if escaped => {
                if ch != SEPARATOR && ch != ESCAPE {
                    return Err(StorePathError::DanglingEscape);
                }
                level_len += 1;
                escaped = false;
            }
            ESCAPE => escaped = true,
            SEPARATOR => {
                if level_len == 0 {
                    return Err(StorePathError::EmptySegment { at: at_level });
                }
                at_level += 1;
                level_len = 0;
            }
            _ => level_len += 1,
        }
    }

    if escaped {
        return Err(StorePathError::DanglingEscape);
    }

    if !joined.is_empty() && level_len == 0 {
        return Err(StorePathError::EmptySegment { at: at_level });
    }

    Ok(())
}

/// One level as it was written, borrowing when nothing was escaped.
fn unescape(level: &str) -> Cow<'_, str> {
    if !level.contains(ESCAPE) {
        return Cow::Borrowed(level);
    }

    let mut out = String::with_capacity(level.len());
    let mut escaped = false;
    for ch in level.chars() {
        match ch {
            _ if escaped => {
                out.push(ch);
                escaped = false;
            }
            ESCAPE => escaped = true,
            _ => out.push(ch),
        }
    }
    Cow::Owned(out)
}

/// Why a set of segments is not a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorePathError {
    /// A level with no name, and which one. It would be indistinguishable from
    /// the root once joined, and there is nothing a store could address by it.
    EmptySegment { at: usize },

    /// An escape that escapes nothing. No key this type wrote holds one, and
    /// reading it leniently would let two different keys name one path.
    DanglingEscape,

    /// No levels at all.
    ///
    /// A path is built from a list, and a list computed at run time can come
    /// out empty - a filter that removed everything, a split of an empty
    /// string. That named the root, so a write through it replaced the whole
    /// store and returned `Ok`, in code that never mentions the root.
    ///
    /// [`StorePath::root`] is how to say it on purpose.
    EmptyPath,
}

impl fmt::Display for StorePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorePathError::EmptySegment { at } => {
                write!(f, "level {at} of the path has no name")
            }
            StorePathError::DanglingEscape => {
                f.write_str("an escape must be followed by a separator or another escape")
            }
            StorePathError::EmptyPath => f.write_str(
                "a path of no levels names the whole store - say `StorePath::root()` to mean it",
            ),
        }
    }
}

impl std::error::Error for StorePathError {}

/// What a call can be given where a path is wanted.
///
/// A list of levels, or a path already built. Deliberately not `&str`: a string
/// is a name, and letting one stand in for a path is the confusion this type
/// exists to end - `store.get(["ui", "width"])` and `store.get("ui.width")`
/// would otherwise look alike and mean different things.
#[diagnostic::on_unimplemented(
    message = "a path is a list of levels, and `{Self}` is not one",
    label = "expected something like `[\"ui\", \"width\"]`",
    note = "a string is a name, not a path: `\"ui.width\"` would be one level \
            whose name holds a separator, which is somewhere else entirely. \
            Spell the nesting out, or use `StorePath::parse_joined` to read a \
            path out of a string on purpose",
    note = "addressing one name rather than a path is what `Kv` is for, and it \
            does take `&str`"
)]
pub trait IntoStorePath {
    fn into_store_path(self) -> Result<StorePath, StorePathError>;
}

impl IntoStorePath for StorePath {
    fn into_store_path(self) -> Result<StorePath, StorePathError> {
        Ok(self)
    }
}

impl IntoStorePath for &StorePath {
    fn into_store_path(self) -> Result<StorePath, StorePathError> {
        Ok(self.clone())
    }
}

impl<S: AsRef<str>, const N: usize> IntoStorePath for [S; N] {
    fn into_store_path(self) -> Result<StorePath, StorePathError> {
        StorePath::try_from_segments(self)
    }
}

impl<S: AsRef<str>> IntoStorePath for &[S] {
    fn into_store_path(self) -> Result<StorePath, StorePathError> {
        StorePath::try_from_segments(self)
    }
}

impl<S: AsRef<str>> IntoStorePath for Vec<S> {
    fn into_store_path(self) -> Result<StorePath, StorePathError> {
        StorePath::try_from_segments(self)
    }
}

const SEPARATOR_BYTE: u8 = SEPARATOR as u8;
const ESCAPE_BYTE: u8 = ESCAPE as u8;

/// Ends a level, and occurs nowhere else: a level's own `0x00` is written as
/// [`STANDS_IN`] `0x01`, so an encoded level holds no byte below `0x01` at all.
///
/// That is what makes a subtree a byte prefix and nothing more. An escape that
/// merely *followed* the terminator would not: `["ui\0x"]` would begin with the
/// encoding of `["ui"]` and read as a key under it.
const ENDS_A_LEVEL: u8 = 0x00;

/// Stands in front of a byte the encoding needs back, and is written the same
/// way itself.
///
/// `0x00` becomes `0x01 0x01` and `0x01` becomes `0x01 0x02`, which keeps the
/// order the bytes were in: both land under `0x02`, where they belong, and
/// above each other in the right order.
const STANDS_IN: u8 = 0x01;

/// A path as a flat engine stores it: every level, in order, each one ended.
///
/// The engines that hold a path as one key had no way to see where a level
/// ended, so the key carried the levels joined by a separator and every name
/// escaped to keep the separator out of it - and escaping does not preserve
/// order, which is why the names had to be compared through a function of their
/// own and why a scan's upper bound had to be guessed at and then filtered.
///
/// A terminator that cannot occur inside a level says the same thing without
/// any of that. Byte order over these keys *is* the order of the level lists
/// they came from, so a subtree is a byte prefix, exactly and with nothing left
/// over to check.
/// Held inline up to the length a path of the depth this library is built for
/// reaches: eight levels with names of ordinary length come to about sixty
/// bytes, and a key is built once per read and once per write rather than once
/// per scan. Past that it spills to the heap and costs what any buffer would.
pub type KeyBytes = SmallVec<[u8; 64]>;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(KeyBytes);

impl StorePath {
    /// This path as a key.
    pub fn key(&self) -> Key {
        let mut out = KeyBytes::new();
        self.write_key(&mut out);
        Key(out)
    }

    /// The same, into a buffer the caller keeps between keys.
    pub fn write_key(&self, out: &mut KeyBytes) {
        for level in self.segments() {
            for &byte in level.as_str().as_bytes() {
                match byte {
                    ENDS_A_LEVEL => out.extend_from_slice(&[STANDS_IN, STANDS_IN]),
                    STANDS_IN => out.extend_from_slice(&[STANDS_IN, STANDS_IN + 1]),
                    byte => out.push(byte),
                }
            }
            out.push(ENDS_A_LEVEL);
        }
    }
}

impl Key {
    /// The root, which every key is under and which is no bytes at all.
    pub fn root() -> Self {
        Key(KeyBytes::new())
    }

    /// A key as a store hands it back.
    ///
    /// Takes the bytes as they are; [`Key::path`] is where a key that this type
    /// did not write is refused, because that is where the answer has somewhere
    /// to go.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Key(KeyBytes::from_slice(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The path this key spells.
    ///
    /// Refuses what this type does not write: bytes that are not text, a level
    /// left unterminated, a level with no name, and an escape that escapes
    /// nothing. Reading any of those leniently would let two keys name one
    /// path.
    pub fn path(&self) -> Result<StorePath, StorePathError> {
        let mut names: Vec<SmolStr> = Vec::new();
        let mut level = KeyBytes::new();
        let mut at = 0;

        while at < self.0.len() {
            let byte = self.0[at];
            at += 1;

            match byte {
                ENDS_A_LEVEL => {
                    let Ok(name) = std::str::from_utf8(&level) else {
                        return Err(not_a_key_this_wrote());
                    };

                    if name.is_empty() {
                        return Err(a_level_with_no_name(names.len()));
                    }

                    names.push(SmolStr::new(name));
                    level.clear();
                }
                STANDS_IN => {
                    let held = self.0.get(at).copied();
                    at += 1;

                    match held {
                        Some(STANDS_IN) => level.push(ENDS_A_LEVEL),
                        Some(byte) if byte == STANDS_IN + 1 => level.push(STANDS_IN),
                        _ => return Err(not_a_key_this_wrote()),
                    }
                }
                byte => level.push(byte),
            }
        }

        if !level.is_empty() {
            return Err(not_a_key_this_wrote());
        }

        match names.is_empty() {
            true => Ok(StorePath::root()),
            false => Ok(StorePath::from_checked(names)),
        }
    }

    /// Whether this key names `prefix` or something under it.
    ///
    /// One test, because a level's terminator cannot occur inside a level: a
    /// key under `pot` starts with `pot\0`, `potato` does not, and there is no
    /// third answer for a caller to filter afterwards.
    pub fn under(&self, prefix: &Key) -> bool {
        self.0.starts_with(&prefix.0)
    }

    /// The half-open range this key's subtree occupies, both ends exact. The
    /// root has no top.
    ///
    /// The top is this key with its last byte raised, which reaches everything
    /// under it because that byte is a terminator - the lowest there is, so
    /// raising it lands past every level the key could hold and short of its
    /// next sibling.
    pub fn subtree(&self) -> (&[u8], Option<KeyBytes>) {
        let mut top = self.0.clone();

        while let Some(last) = top.pop() {
            if last < u8::MAX {
                top.push(last + 1);
                return (&self.0, Some(top));
            }
        }

        (&[], None)
    }
}

/// Refusals a decode makes, kept out of line.
///
/// Every key a store hands back is one this library wrote, so these arms are
/// taken by a hand edit or an older build and by nothing else - and a decode
/// runs once per key of every scan. `#[cold]` is what says so on a stable
/// compiler: `hint::cold_path` is not one yet.
#[cold]
#[inline(never)]
fn not_a_key_this_wrote() -> StorePathError {
    StorePathError::DanglingEscape
}

#[cold]
#[inline(never)]
fn a_level_with_no_name(at: usize) -> StorePathError {
    StorePathError::EmptySegment { at }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.path() {
            Ok(path) => write!(f, "Key({path})"),
            Err(_) => write!(f, "Key({:?})", &self.0),
        }
    }
}

impl From<&StorePath> for Key {
    fn from(path: &StorePath) -> Self {
        path.key()
    }
}

const fn check_static(segments: &[&str], joined: &str) {
    if has_empty_segment(segments) {
        panic!("a path segment cannot be empty");
    }

    if !joins_to(segments, joined) {
        panic!(
            "the joined form of a static path must be what StorePath::as_str writes for its segments"
        );
    }
}

const fn has_empty_segment(segments: &[&str]) -> bool {
    let mut at = 0;

    while at < segments.len() {
        if segments[at].is_empty() {
            return true;
        }
        at += 1;
    }

    false
}

const fn joins_to(segments: &[&str], joined: &str) -> bool {
    let key = joined.as_bytes();
    let mut at = 0;
    let mut i = 0;

    while at < segments.len() {
        if at > 0 {
            if i >= key.len() || key[i] != SEPARATOR_BYTE {
                return false;
            }
            i += 1;
        }

        let segment = segments[at].as_bytes();
        let mut j = 0;

        while j < segment.len() {
            let byte = segment[j];

            if byte == SEPARATOR_BYTE || byte == ESCAPE_BYTE {
                if i >= key.len() || key[i] != ESCAPE_BYTE {
                    return false;
                }
                i += 1;
            }

            if i >= key.len() || key[i] != byte {
                return false;
            }
            i += 1;
            j += 1;
        }

        at += 1;
    }

    i == key.len()
}

fn join(segments: &[SmolStr]) -> SmolStr {
    let mut out = String::new();

    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            out.push(SEPARATOR);
        }
        for ch in segment.chars() {
            if ch == SEPARATOR || ch == ESCAPE {
                out.push(ESCAPE);
            }
            out.push(ch);
        }
    }

    SmolStr::new(out)
}

/// Equal paths are the ones that address the same place, which is the ones
/// holding the same levels - the same question [`Ord`] answers, asked of the
/// same form, so the two cannot drift apart.
impl PartialEq for StorePath {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.segments().eq(other.segments())
    }
}

impl Eq for StorePath {}

/// The order a store lists in: by the levels, outermost first.
///
/// The flat engines range over [`Key`], whose byte order is this order, so
/// anything sorting paths for itself agrees with them by construction.
///
/// Not by the joined spelling. Joining puts a separator between levels and
/// escapes it inside them, and a name holding a byte below that separator then
/// sorts on the wrong side of a boundary: `["a!"]` comes after `["a", "b"]` by
/// levels and before it by spelling. The levels are what a path *is*, and
/// [`Key`] encodes them so that a flat engine's byte order says the same thing.
impl Ord for StorePath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.segments().cmp(other.segments())
    }
}

impl PartialOrd for StorePath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// By the levels, so it agrees with `Eq` however the path arrived.
///
/// `str`'s own `Hash` ends what it writes, so two levels cannot run together
/// into the hash of one - `["a", "b"]` and `["ab"]` are different paths and
/// hash differently.
impl Hash for StorePath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        for level in self.segments() {
            level.as_str().hash(state);
        }
    }
}

impl fmt::Debug for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StorePath({})", self)
    }
}

/// The joined spelling, written a level at a time rather than built and then
/// written: printing a path is not a reason to make one hold a second form of
/// itself.
impl fmt::Display for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (at, level) in self.segments().enumerate() {
            if at > 0 {
                f.write_char(SEPARATOR)?;
            }

            for ch in level.as_str().chars() {
                if ch == SEPARATOR || ch == ESCAPE {
                    f.write_char(ESCAPE)?;
                }
                f.write_char(ch)?;
            }
        }

        Ok(())
    }
}

/// A path both of whose halves the compiler knows: the levels and the joined
/// form, checked against each other where they are written.
///
/// [`StorePath`] holds a run-time path behind an `Arc`, which costs two things
/// a declaration cannot pay. It has a destructor, so a `static` built from one
/// cannot use `..` to fill in the rest of a struct; and its joined form cannot
/// be read in a `const`, which is where the checks over a declaration run.
/// This type has neither: it is `Copy`, it owns nothing, and everything about
/// it is available while the code is compiled.
///
/// It is what a declaration carries. Turn it into a `StorePath` with
/// [`StaticPath::path`] wherever a run-time path is wanted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StaticPath {
    segments: &'static [&'static str],
    joined: &'static str,
}

impl StaticPath {
    /// # Panics
    ///
    /// If any level is empty, or if `joined` is not the joined form of
    /// `segments` - at compile time, which is the only place this is meant to
    /// be called. See [`StorePath::from_static`], which checks the same pair.
    pub const fn new(segments: &'static [&'static str], joined: &'static str) -> Self {
        check_static(segments, joined);

        Self { segments, joined }
    }

    /// The path that is under nothing.
    pub const fn root() -> Self {
        Self {
            segments: &[],
            joined: "",
        }
    }

    /// The whole path as one string, where a `const` can read it.
    pub const fn as_str(&self) -> &'static str {
        self.joined
    }

    pub const fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// The same path, as the type the store addresses by. Allocates nothing:
    /// both halves are already `'static`.
    pub fn path(&self) -> StorePath {
        StorePath::from_static(self.segments, self.joined)
    }
}

impl From<StaticPath> for StorePath {
    fn from(at: StaticPath) -> Self {
        at.path()
    }
}

impl PartialEq<str> for StaticPath {
    fn eq(&self, other: &str) -> bool {
        self.joined == other
    }
}

impl AsRef<str> for StaticPath {
    fn as_ref(&self) -> &str {
        self.joined
    }
}

impl fmt::Debug for StaticPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StaticPath({:?})", self.joined)
    }
}

impl fmt::Display for StaticPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.joined)
    }
}

/// Written as the joined form, which is the only form a document holds.
impl serde::Serialize for StorePath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Read back through [`StorePath::parse_joined`], so the invariants are checked
/// where the document is read rather than wherever somebody first asks for a
/// level: a key with a nameless level or a dangling escape is refused here and
/// cannot reach anything that would have to answer for it later.
impl<'de> serde::Deserialize<'de> for StorePath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let joined = <Cow<'de, str>>::deserialize(deserializer)?;
        StorePath::parse_joined(&joined).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn shares_a_key_by_levels(a: &StorePath, b: &StorePath) -> bool {
        let a: Vec<_> = a.segments().collect();
        let b: Vec<_> = b.segments().collect();
        let common = a.len().min(b.len());
        a[..common] == b[..common]
    }

    #[test]
    fn two_subtrees_overlap_exactly_when_one_holds_the_other() {
        let cases: &[(&[&str], &[&str], bool, &str)] = &[
            (&["ui"], &["ui"], true, "the same path"),
            (&["ui"], &["ui", "theme"], true, "a child"),
            (
                &["ui"],
                &["ui", "a", "b"],
                true,
                "a grandchild - a subtree is not one level",
            ),
            (&["ui", "theme"], &["ui", "width"], false, "siblings"),
            (&["ui"], &["net"], false, "strangers"),
            (
                &["ui"],
                &["uix"],
                false,
                "a string prefix is not a level prefix",
            ),
            (
                &["ui"],
                &["uix", "width"],
                false,
                "and neither is its subtree",
            ),
            (
                &["ui"],
                &["ui!x"],
                false,
                "`!` sorts below the separator, and is still not under `ui`",
            ),
            (
                &["ui.theme"],
                &["ui"],
                false,
                "one level literally named `ui.theme`, escaped as `ui\\.theme`",
            ),
            (
                &["ui.theme"],
                &["ui", "theme"],
                false,
                "which is a different place from two levels",
            ),
            (&[], &["ui", "theme"], true, "the root holds everything"),
        ];

        for (a, b, want, why) in cases {
            let (a, b) = (StorePath::from_segments(*a), StorePath::from_segments(*b));
            assert_eq!(a.overlaps(&b), *want, "{why}: {a} vs {b}");
            assert_eq!(
                b.overlaps(&a),
                *want,
                "{why}, the other way round: {b} vs {a}"
            );
        }
    }

    use crate::strategies::{key as key_strategy, name_holding_the_separator, segment};

    fn segment_strategy() -> impl Strategy<Value = String> {
        segment()
    }

    fn dotted_segment() -> impl Strategy<Value = String> {
        name_holding_the_separator().prop_map(|(_, _, joined)| joined)
    }

    fn path_strategy() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(segment_strategy(), 1..5)
    }

    fn hash_of(value: &impl Hash) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    proptest! {
        #[test]
        fn two_subtrees_meet_exactly_when_one_starts_the_other(
            a in path_strategy(),
            b in path_strategy(),
        ) {
            let (a, b) = (StorePath::from_segments(&a), StorePath::from_segments(&b));

            prop_assert_eq!(
                a.overlaps(&b),
                shares_a_key_by_levels(&a, &b),
                "the joined form and the levels disagree: {} vs {}", a, b
            );
            prop_assert_eq!(a.overlaps(&b), b.overlaps(&a), "{} vs {}", a, b);

            // Nested means the deeper one is itself the shared key.
            if a.overlaps(&b) {
                let deeper = if a.len() >= b.len() { &a } else { &b };
                prop_assert!(deeper.starts_with(&a));
                prop_assert!(deeper.starts_with(&b));
            }
        }

        #[test]
        fn a_path_meets_everything_grown_from_it(
            head in path_strategy(),
            tail in prop::collection::vec(segment_strategy(), 0..4),
        ) {
            let a = StorePath::from_segments(&head);
            let b = StorePath::from_segments(head.iter().chain(&tail));

            prop_assert!(a.overlaps(&b), "{} does not hold {}", a, b);
            prop_assert!(b.starts_with(&a), "{} vs {}", a, b);
        }

        #[test]
        fn two_branches_of_one_level_never_meet(
            head in path_strategy(),
            left in segment_strategy(),
            right in segment_strategy(),
            left_tail in prop::collection::vec(segment_strategy(), 0..3),
            right_tail in prop::collection::vec(segment_strategy(), 0..3),
        ) {
            prop_assume!(left != right);

            let a = StorePath::from_segments(head.iter().chain([&left]).chain(&left_tail));
            let b = StorePath::from_segments(head.iter().chain([&right]).chain(&right_tail));

            prop_assert!(!a.overlaps(&b), "siblings met: {} vs {}", a, b);
        }

        #[test]
        fn a_separator_inside_a_name_is_never_a_level(
            segments in prop::collection::vec(dotted_segment(), 1..16)
        ) {
            let path = StorePath::from_segments(&segments);

            prop_assert_eq!(path.len(), segments.len());
            prop_assert_eq!(
                path.segments().map(|s| s.to_string()).collect::<Vec<_>>(),
                segments.clone()
            );

            let split: Vec<&str> = segments.iter().flat_map(|s| s.split('.')).collect();
            if let Ok(taken_apart) = StorePath::try_from_segments(&split) {
                prop_assert_ne!(taken_apart, path);
            }
        }
        #[test]
        fn the_joined_form_round_trips(segments in path_strategy()) {
            let path = StorePath::from_segments(&segments);
            prop_assert_eq!(StorePath::parse_joined(path.as_str()).unwrap(), path);
        }

        #[test]
        fn different_levels_never_join_to_one_key(a in path_strategy(), b in path_strategy()) {
            let pa = StorePath::from_segments(&a);
            let pb = StorePath::from_segments(&b);

            prop_assert_eq!(a == b, pa.as_str() == pb.as_str());
        }

        #[test]
        fn a_prefix_is_stripped_back_off(head in path_strategy(), tail in path_strategy()) {
            let prefix = StorePath::from_segments(&head);
            let full = prefix.join(&StorePath::from_segments(&tail));

            prop_assert!(full.starts_with(&prefix));
            prop_assert_eq!(
                full.strip_prefix(&prefix).unwrap(),
                StorePath::from_segments(&tail)
            );
        }

        #[test]
        fn a_path_does_not_remember_how_it_was_built(segments in path_strategy()) {
            let all_at_once = StorePath::from_segments(&segments);

            let mut one_at_a_time = StorePath::root();
            for segment in &segments {
                one_at_a_time = one_at_a_time.push(segment);
            }

            let by_joining = segments.iter().fold(StorePath::root(), |acc, segment| {
                acc.join(&StorePath::segment(segment))
            });

            prop_assert_eq!(&one_at_a_time, &all_at_once);
            prop_assert_eq!(&by_joining, &all_at_once);
            prop_assert_eq!(one_at_a_time.as_str(), all_at_once.as_str());
        }

        #[test]
        fn any_spelling_of_the_levels_gives_the_same_path(segments in path_strategy()) {
            let built = StorePath::from_segments(&segments);
            let refs: Vec<&str> = segments.iter().map(String::as_str).collect();

            prop_assert_eq!(refs.as_slice().into_store_path().unwrap(), built.clone());
            prop_assert_eq!(segments.clone().into_store_path().unwrap(), built.clone());
            prop_assert_eq!((&built).into_store_path().unwrap(), built.clone());
            prop_assert_eq!(built.clone().into_store_path().unwrap(), built);
        }

        #[test]
        fn every_path_is_under_the_root(segments in path_strategy()) {
            let path = StorePath::from_segments(&segments);
            let root = StorePath::root();

            prop_assert!(path.starts_with(&root));
            prop_assert_eq!(path.strip_prefix(&root).unwrap(), path.clone());
            prop_assert!(!root.starts_with(&path));
        }

        #[test]
        fn a_level_with_no_name_is_not_a_path(
            segments in path_strategy(),
            at in 0usize..8
        ) {
            let mut with_a_hole = segments.clone();
            let at = at % (with_a_hole.len() + 1);
            with_a_hole.insert(at, String::new());

            prop_assert_eq!(
                StorePath::try_from_segments(&with_a_hole),
                Err(StorePathError::EmptySegment { at }),
                "the refusal names the level that has no name"
            );
            prop_assert_eq!(
                StorePath::from_segments(&segments).try_push(""),
                Err(StorePathError::EmptySegment { at: segments.len() })
            );
        }

        #[test]
        fn every_separator_inside_a_name_is_escaped(segments in path_strategy()) {
            let path = StorePath::from_segments(&segments);

            let mut unescaped = 0usize;
            let mut escaped = false;
            for ch in path.as_str().chars() {
                match ch {
                    _ if escaped => escaped = false,
                    ESCAPE => escaped = true,
                    SEPARATOR => unescaped += 1,
                    _ => {}
                }
            }

            prop_assert_eq!(unescaped, segments.len() - 1);
        }

        #[test]
        fn growing_a_name_never_makes_it_a_prefix(
            head in path_strategy(),
            extra in segment_strategy()
        ) {
            let base = StorePath::from_segments(&head);

            let mut grown = head.clone();
            grown.last_mut().unwrap().push_str(&extra);
            let longer = StorePath::from_segments(&grown);

            prop_assert!(longer.as_str().starts_with(base.as_str()));

            prop_assert_ne!(&longer, &base);
            prop_assert!(!longer.starts_with(&base));
            prop_assert!(!base.starts_with(&longer));
        }

        #[test]
        fn stripping_succeeds_exactly_when_the_prefix_matches(
            a in path_strategy(),
            b in path_strategy()
        ) {
            let path = StorePath::from_segments(&a);
            let candidate = StorePath::from_segments(&b);

            prop_assert_eq!(
                path.strip_prefix(&candidate).is_some(),
                path.starts_with(&candidate)
            );
        }

        #[test]
        fn a_key_that_parses_joins_back_to_itself(key in key_strategy()) {
            if let Ok(path) = StorePath::parse_joined(&key) {
                prop_assert_eq!(path.as_str(), key.as_str());
            }
        }

        #[test]
        fn a_key_with_a_nameless_level_is_refused(
            head in path_strategy(),
            tail in path_strategy(),
            at in 0usize..3
        ) {
            let head = StorePath::from_segments(&head);
            let tail = StorePath::from_segments(&tail);

            let (key, hole) = match at % 3 {
                0 => (format!("{SEPARATOR}{tail}"), 0),
                1 => (format!("{head}{SEPARATOR}"), head.len()),
                _ => (
                    format!("{head}{SEPARATOR}{SEPARATOR}{tail}"),
                    head.len(),
                ),
            };

            prop_assert_eq!(
                StorePath::parse_joined(&key),
                Err(StorePathError::EmptySegment { at: hole }),
                "key: {:?}", key
            );
        }


        #[test]
        fn a_borrowed_path_answers_like_the_owned_one(
            key in path_strategy(),
            prefix in path_strategy()
        ) {
            let owned = StorePath::from_segments(&key);
            let under = StorePath::from_segments(&prefix);
            let borrowed = PathRef::from(&owned);

            prop_assert_eq!(borrowed.as_str(), owned.as_str());
            prop_assert_eq!(borrowed.is_root(), owned.is_root());
            prop_assert_eq!(borrowed.level_under(&under), owned.level_under(&under));
            prop_assert_eq!(borrowed.name_under(&under), owned.name_under(&under));
            prop_assert_eq!(borrowed.to_path(), owned);
        }

        #[test]
        fn a_key_borrows_exactly_when_it_parses(key in key_strategy()) {
            prop_assert_eq!(
                PathRef::parse(&key).is_ok(),
                StorePath::parse_joined(&key).is_ok(),
                "key: {:?}", key
            );

            if let Ok(borrowed) = PathRef::parse(&key) {
                let owned = borrowed.to_path();

                prop_assert_eq!(borrowed.as_str(), key.as_str());
                prop_assert_eq!(owned.as_str(), key.as_str());
            }
        }

        #[test]
        fn the_range_a_key_gives_holds_exactly_the_subtree(
            head in path_strategy(),
            other in path_strategy()
        ) {
            let prefix = StorePath::from_segments(&head);
            let key = StorePath::from_segments(&other);

            let under = prefix.key();
            let (low, high) = under.subtree();
            let bytes = key.key();
            let bytes = bytes.as_bytes();

            let in_range = bytes >= low && high.as_ref().is_none_or(|h| bytes < h.as_slice());

            prop_assert_eq!(
                in_range,
                key.starts_with(&prefix),
                "{} against the subtree of {}", key, prefix
            );
        }

        #[test]
        fn everything_grown_from_a_path_is_in_its_range(
            head in path_strategy(),
            tail in prop::collection::vec(segment_strategy(), 0..4)
        ) {
            let prefix = StorePath::from_segments(&head);
            let under = prefix.join(&StorePath::from_segments(&tail));

            prop_assert!(
                under.key().under(&prefix.key()),
                "{} is under {} and the range left it out", under, prefix
            );
        }

        #[test]
        fn a_key_is_under_a_path_exactly_when_it_starts_with_it_and_a_separator(
            head in path_strategy(),
            tail in path_strategy(),
            other in path_strategy()
        ) {
            let prefix = StorePath::from_segments(&head);
            let boundary = format!("{}{}", prefix.as_str(), SEPARATOR);

            let candidates = [
                prefix.join(&StorePath::from_segments(&tail)),
                StorePath::from_segments(&other),
                prefix.clone(),
                StorePath::from_segments(&tail).join(&prefix),
            ];

            for candidate in candidates {
                let under = candidate.starts_with(&prefix) && candidate != prefix;
                prop_assert_eq!(under, candidate.as_str().starts_with(&boundary));
            }
        }

        #[test]
        fn no_call_makes_a_path_from_segments_would_refuse(
            a in path_strategy(),
            b in path_strategy()
        ) {
            let pa = StorePath::from_segments(&a);
            let pb = StorePath::from_segments(&b);

            let mut derived = vec![
                pa.join(&pb),
                pa.push("x"),
                StorePath::parse_joined(pa.as_str()).unwrap(),
                pa.join(&StorePath::root()),
            ];
            derived.extend(pa.parent());
            derived.extend(pa.join(&pb).strip_prefix(&pa));

            for path in derived {
                let rebuilt = match StorePath::try_from_segments(path.segments()) {
                    Ok(rebuilt) => rebuilt,
                    Err(StorePathError::EmptyPath) => {
                        prop_assert!(path.is_root());
                        continue;
                    }
                    Err(other) => {
                        return Err(TestCaseError::fail(format!(
                            "a derived path holds an empty level: {other}"
                        )));
                    }
                };
                prop_assert_eq!(&rebuilt, &path);
                prop_assert_eq!(rebuilt.as_str(), path.as_str());
            }
        }

        #[test]
        fn comparing_names_answers_what_comparing_their_keys_answers(
            a in segment_strategy(),
            b in segment_strategy()
        ) {
            let under = StorePath::segment("m");
            let ka = under.push(&a);
            let kb = under.push(&b);

            prop_assert_eq!(
                a.cmp(&b),
                ka.key().as_bytes().cmp(kb.key().as_bytes()),
                "names {:?} and {:?} became keys {:?} and {:?}",
                a, b, ka, kb
            );
        }

        #[test]
        fn equality_hashing_and_the_key_agree(a in path_strategy(), b in path_strategy()) {
            let pa = StorePath::from_segments(&a);
            let pb = StorePath::from_segments(&b);

            let left = [pa.clone(), pa.parent().unwrap_or_else(StorePath::root), StorePath::root()];
            let right = [pb.clone(), pa.join(&pb).strip_prefix(&pa).unwrap(), StorePath::root()];

            for x in &left {
                for y in &right {
                    prop_assert_eq!(x == y, x.as_str() == y.as_str());
                    if x == y {
                        prop_assert_eq!(hash_of(x), hash_of(y));
                    }
                }
            }
        }

        #[test]
        fn the_const_check_accepts_what_the_join_writes(segments in path_strategy()) {
            let path = StorePath::from_segments(&segments);
            let levels: Vec<&str> = segments.iter().map(String::as_str).collect();

            prop_assert!(
                joins_to(&levels, path.as_str()),
                "refused its own join: {:?}", path.as_str()
            );

            let joined = path.as_str();
            let perturbed = [
                format!("{joined}{SEPARATOR}"),
                format!("{SEPARATOR}{joined}"),
                format!("{joined}{ESCAPE}"),
                format!("{joined}x"),
            ];

            for candidate in perturbed {
                prop_assert!(
                    !joins_to(&levels, &candidate),
                    "accepted {:?} as the join of {:?}", candidate, levels
                );
            }

            if levels.len() > 1 {
                let shorter = StorePath::from_segments(&segments[..segments.len() - 1]);
                prop_assert!(!joins_to(&levels, shorter.as_str()));
            }
        }
    }

    #[test]
    fn the_encoding_is_a_backslash_before_the_separator() {
        assert_eq!(StorePath::segment("dark.mode").as_str(), "dark\\.mode");
        assert_eq!(
            StorePath::from_segments(["dark", "mode"]).as_str(),
            "dark.mode"
        );
    }

    #[test]
    fn a_key_that_is_not_a_path_is_refused_at_both_doors() {
        let refused = [
            ("a.", StorePathError::EmptySegment { at: 1 }),
            (".a", StorePathError::EmptySegment { at: 0 }),
            ("a..b", StorePathError::EmptySegment { at: 1 }),
            (".", StorePathError::EmptySegment { at: 0 }),
            ("a\\", StorePathError::DanglingEscape),
            ("a\\x", StorePathError::DanglingEscape),
        ];

        for (key, why) in refused {
            assert_eq!(
                StorePath::parse_joined(key).unwrap_err(),
                why,
                "parse_joined took {key:?}"
            );
            assert!(
                PathRef::parse(key).is_err(),
                "PathRef::parse took {key:?}, and the two doors must agree"
            );
        }
    }

    #[test]
    fn a_subtree_is_one_run_with_nothing_of_anyone_elses_in_it() {
        let pot = StorePath::segment("pot");
        let luck = StorePath::segment("pot!luck");
        let ato = StorePath::from_segments(["pot", "ato"]);
        let potato = StorePath::segment("potato");

        let mut sorted = [&potato, &luck, &ato, &pot];
        sorted.sort();

        assert_eq!(
            sorted,
            [&pot, &ato, &luck, &potato],
            "everything under `pot` has to come directly after it"
        );

        assert!(ato.starts_with(&pot));
        assert!(!luck.starts_with(&pot));
        assert!(!potato.starts_with(&pot));
    }

    #[test]
    fn a_shared_prefix_answers_as_the_path_it_spells() {
        let full = StorePath::from_segments(["ui", "panels", "left", "width"]);

        for (up, spelled) in [
            (1, "ui.panels.left"),
            (2, "ui.panels"),
            (3, "ui"),
            (4, ""),
        ] {
            let mut at = full.clone();
            for _ in 0..up {
                at = at.parent().expect("there is a level to drop");
            }

            let built = StorePath::parse_joined(spelled).expect("the spelling parses");

            assert_eq!(at.as_str(), spelled);
            assert_eq!(at, built);
            assert_eq!(at.len(), built.len());
            assert_eq!(at.is_root(), built.is_root());
            assert_eq!(at.key().as_bytes(), built.key().as_bytes());
            assert_eq!(
                at.segments().collect::<Vec<_>>(),
                built.segments().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_prefix_of_a_name_holding_the_separator_cuts_where_the_level_does() {
        let full = StorePath::from_segments(["dark.mode", "width"]);
        let up = full.parent().expect("there is a level to drop");

        assert_eq!(up.len(), 1);
        assert_eq!(up.name().expect("one level").as_str(), "dark.mode");
        assert_eq!(up.as_str(), "dark\\.mode");
        assert_eq!(up, StorePath::segment("dark.mode"));
    }

    #[test]
    fn walking_upwards_ends_at_the_root_and_holds_the_path_itself() {
        let full = StorePath::from_segments(["a", "b", "c"]);

        let seen: Vec<String> = full.upwards().map(|at| at.as_str().to_string()).collect();

        assert_eq!(seen, ["a.b.c", "a.b", "a", ""]);
        assert_eq!(StorePath::root().upwards().count(), 1);
    }

    #[test]
    fn the_root_is_empty_and_stays_empty() {
        let root = StorePath::root();

        assert!(root.is_root());
        assert_eq!(root.as_str(), "");
        assert_eq!(StorePath::parse_joined("").unwrap(), root);
        assert_eq!(root.parent(), None);
        assert_eq!(root.name(), None);
    }

    #[test]
    fn a_key_no_join_could_have_written_is_not_a_second_name_for_one() {
        assert_ne!(
            StorePath::parse_joined("a\\b").ok(),
            StorePath::parse_joined("ab").ok()
        );
        assert_ne!(
            StorePath::parse_joined("a\\").ok(),
            StorePath::parse_joined("a").ok()
        );
    }

    #[test]
    fn the_root_has_no_separator_boundary() {
        let root = StorePath::root();
        let child = StorePath::from_segments(["ui", "width"]);

        assert!(child.starts_with(&root));
        assert_eq!(format!("{}{}", root.as_str(), SEPARATOR), ".");
        assert!(!child.as_str().starts_with(SEPARATOR));
        assert!(
            !StorePath::segment(".hidden")
                .as_str()
                .starts_with(SEPARATOR)
        );
    }

    #[test]
    fn a_subtree_boundary_does_not_cover_the_node_itself() {
        let node = StorePath::segment("ui");
        let boundary = format!("{}{}", node.as_str(), SEPARATOR);

        assert!(node.push("width").as_str().starts_with(&boundary));
        assert!(!node.as_str().starts_with(&boundary));
        assert!(node.starts_with(&node));
    }

    #[test]
    fn a_key_carries_what_a_glob_pattern_reads() {
        let path = StorePath::segment("a*b[c]?\u{0}d");

        assert_eq!(path.as_str(), "a*b[c]?\u{0}d");
    }

    #[test]
    fn a_path_hashes_by_its_levels_however_it_arrived() {
        let built = StorePath::from_segments(["ui", "dark.mode"]);
        let parsed = StorePath::parse_joined("ui.dark\\.mode").expect("the spelling parses");

        assert_eq!(built, parsed);
        assert_eq!(hash_of(&built), hash_of(&parsed));
    }

    #[test]
    fn two_levels_do_not_hash_as_the_one_they_spell() {
        let two = StorePath::from_segments(["a", "b"]);
        let one = StorePath::segment("ab");

        assert_ne!(two, one);
        assert_ne!(hash_of(&two), hash_of(&one));
    }

    #[test]
    fn a_path_flattened_to_one_level_is_named_by_what_it_spells() {
        let flat = StorePath::from_segments(["ui", "theme"]).as_one_level();

        assert_eq!(flat.len(), 1);
        assert_eq!(flat.name().expect("one level has a name"), "ui.theme");
    }

    #[test]
    fn a_level_holding_a_separator_flattens_apart_from_the_two_it_reads_as() {
        let one = StorePath::segment("ui.theme").as_one_level();
        let two = StorePath::from_segments(["ui", "theme"]).as_one_level();

        assert_ne!(one, two);
        assert_eq!(
            StorePath::parse_joined(one.name().expect("one level has a name").as_str())
                .expect("the spelling parses"),
            StorePath::segment("ui.theme")
        );
    }

    #[test]
    fn two_spellings_of_one_name_are_two_paths() {
        let precomposed = StorePath::segment("caf\u{e9}");
        let decomposed = StorePath::segment("cafe\u{301}");

        assert_ne!(precomposed, decomposed);
        assert_ne!(precomposed.as_str(), decomposed.as_str());
        assert_eq!(precomposed.to_string().chars().count(), 4);
        assert_eq!(decomposed.to_string().chars().count(), 5);
    }

    #[test]
    fn walking_a_document_borrows_each_level() {
        let path = StorePath::from_segments(["ui", "window", "width"]);

        assert_eq!(
            path.segments()
                .map(|l| l.as_str().to_string())
                .collect::<Vec<_>>(),
            ["ui", "window", "width"]
        );
        assert_eq!(
            path.segment_at(1).as_ref().map(Level::as_str),
            Some("window")
        );
        assert_eq!(path.segment_at(3), None);
    }

    #[test]
    fn a_static_path_is_the_same_path() {
        static UI_WIDTH: StorePath = StorePath::from_static(&["ui", "width"], "ui.width");

        assert_eq!(UI_WIDTH, StorePath::from_segments(["ui", "width"]));
        assert_eq!(UI_WIDTH.as_str(), "ui.width");
        assert_eq!(UI_WIDTH.name().as_ref().map(Level::as_str), Some("width"));
        assert!(UI_WIDTH.starts_with(&StorePath::from_static(&["ui"], "ui")));
    }

    #[test]
    fn a_static_path_carries_what_a_name_holds() {
        static ODD: StorePath =
            StorePath::from_static(&["dark.mode", "a\\b"], "dark\\.mode.a\\\\b");
        static NOTHING: StorePath = StorePath::from_static(&[], "");

        assert_eq!(ODD, StorePath::from_segments(["dark.mode", "a\\b"]));
        assert_eq!(
            ODD.as_str(),
            StorePath::from_segments(["dark.mode", "a\\b"]).as_str()
        );
        assert_eq!(NOTHING, StorePath::root());
    }
}
