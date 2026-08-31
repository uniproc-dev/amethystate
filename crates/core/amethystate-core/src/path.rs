use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

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
/// two levels. Both forms are kept - the segments for engines that walk a
/// document tree, and the joined string for engines that store a key whole - so
/// neither costs an allocation to read.
#[derive(Clone)]
pub struct StorePath {
    segments: Segments,
    joined: Joined,
}

#[derive(Clone)]
enum Segments {
    Static(&'static [&'static str]),
    Owned(Arc<[Arc<str>]>),

    /// Not split yet, and possibly never.
    ///
    /// A path read back from a store arrives as one string, and the engines
    /// that read it back that way - the ones storing a key whole - address it
    /// by that string from end to end: equality, order and hashing all answer
    /// from the joined form. Splitting it into a level per `Arc<str>` is an
    /// allocation for the list and one for each level, taken on every key of
    /// every scan, for levels nobody looks at.
    ///
    /// Whoever does look at them walks the string then and carries nothing
    /// away, so a scan pays for levels only where something reads them. The
    /// paths that are walked level by level are the ones the document engines
    /// build *from* levels, and those are `Owned`.
    ///
    /// Which is why a level comes back as a `Cow`: one holding an escaped
    /// separator is not a run of the joined string and has to be assembled,
    /// and one without is borrowed from it.
    Deferred,
}

#[derive(Clone)]
enum Joined {
    Static(&'static str),
    Owned(Arc<str>),
}

impl Segments {
    fn len(&self, joined: &str) -> usize {
        match self {
            Segments::Static(s) => s.len(),
            Segments::Owned(s) => s.len(),
            Segments::Deferred => count_levels(joined),
        }
    }

    fn get<'a>(&'a self, joined: &'a str, index: usize) -> Option<Cow<'a, str>> {
        match self {
            Segments::Static(s) => s.get(index).copied().map(Cow::Borrowed),
            Segments::Owned(s) => s.get(index).map(|s| Cow::Borrowed(&**s)),
            Segments::Deferred => level_at(joined, index),
        }
    }

    fn to_owned_vec(&self, joined: &str) -> Vec<Arc<str>> {
        match self {
            Segments::Static(s) => s.iter().map(|s| Arc::from(*s)).collect(),
            Segments::Owned(s) => s.to_vec(),
            Segments::Deferred => (0..count_levels(joined))
                .map(|i| Arc::from(&*level_at(joined, i).expect("counted")))
                .collect(),
        }
    }
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
            segments: Segments::Static(&[]),
            joined: Joined::Static(""),
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
            segments: Segments::Static(segments),
            joined: Joined::Static(joined),
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
        let mut collected: Vec<Arc<str>> = Vec::new();

        for (at, segment) in segments.into_iter().enumerate() {
            let segment = segment.as_ref();
            if segment.is_empty() {
                return Err(StorePathError::EmptySegment { at });
            }
            collected.push(Arc::from(segment));
        }

        if collected.is_empty() {
            return Err(StorePathError::EmptyPath);
        }

        Ok(Self::from_checked(collected))
    }

    fn from_checked(segments: Vec<Arc<str>>) -> Self {
        let joined = join(&segments);

        Self {
            segments: Segments::Owned(Arc::from(segments)),
            joined: Joined::Owned(joined),
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
        let name = name.as_ref();
        if name.is_empty() {
            return Err(StorePathError::EmptySegment { at: self.len() });
        }

        let mut segments = self.segments.to_owned_vec(self.as_str());
        segments.push(Arc::from(name));
        Ok(Self::from_checked(segments))
    }

    /// This path with `other`'s levels under it.
    pub fn join(&self, other: &StorePath) -> Self {
        let mut segments = self.segments.to_owned_vec(self.as_str());
        segments.extend(other.segments.to_owned_vec(other.as_str()));
        Self::from_checked(segments)
    }

    /// The levels, outermost first.
    ///
    /// Borrowed where the level is a run of the joined form, which is every
    /// level whose name carries no separator to escape.
    pub fn segments(&self) -> impl ExactSizeIterator<Item = Cow<'_, str>> + '_ {
        (0..self.len()).map(|i| self.segment_at(i).expect("counted"))
    }

    /// One level, or `None` past the end.
    pub fn segment_at(&self, index: usize) -> Option<Cow<'_, str>> {
        self.segments.get(self.as_str(), index)
    }

    /// The whole path as one string, with the separator escaped inside names.
    ///
    /// This is what a flat engine stores as its key. Reading it costs nothing:
    /// it is built once, when the path is.
    pub fn as_str(&self) -> &str {
        match &self.joined {
            Joined::Static(s) => s,
            Joined::Owned(s) => s,
        }
    }

    /// This path and everything under it, as the key space sees it. Costs one
    /// string, so a scan builds it once and asks it about every key.
    pub fn subtree(&self) -> Subtree<'_> {
        Subtree {
            prefix: self.as_str(),
            bound: (!self.is_root()).then(|| format!("{}{SEPARATOR}", self.as_str())),
        }
    }

    /// The joined form as a shared handle. A runtime path already holds one; a
    /// path from the macro holds a `&'static str` and makes one.
    pub fn joined_arc(&self) -> Arc<str> {
        match &self.joined {
            Joined::Static(s) => Arc::from(*s),
            Joined::Owned(s) => s.clone(),
        }
    }

    pub fn is_root(&self) -> bool {
        self.as_str().is_empty()
    }

    pub fn len(&self) -> usize {
        self.segments.len(self.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.is_root()
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
        self.starts_with(prefix).then(|| {
            StorePath::from_checked(
                self.segments.to_owned_vec(self.as_str())[prefix.len()..].to_vec(),
            )
        })
    }

    /// The path one level up, or `None` at the root.
    /// Whether this subtree and `other`'s hold any key in common.
    ///
    /// Two subtrees are nested or apart and never half over each other, so this
    /// is two containment tests and equality needs no arm of its own -
    /// [`Subtree::contains`] admits the prefix itself.
    pub fn overlaps(&self, other: &StorePath) -> bool {
        self.subtree().contains(other.as_str()) || other.subtree().contains(self.as_str())
    }

    pub fn parent(&self) -> Option<StorePath> {
        (!self.is_root()).then(|| {
            let mut segments = self.segments.to_owned_vec(self.as_str());
            segments.pop();
            StorePath::from_checked(segments)
        })
    }

    /// The last level, or `None` at the root.
    pub fn name(&self) -> Option<Cow<'_, str>> {
        self.segment_at(self.len().checked_sub(1)?)
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
    pub fn name_under(&self, prefix: &StorePath) -> Option<Cow<'_, str>> {
        match level_below(self.as_str(), prefix.as_str(), prefix.is_root()) {
            Level::Entry(name) | Level::Deeper(name) => Some(name),
            Level::Prefix | Level::Outside => None,
        }
    }

    /// The name `key` is stored under, below this path: the *last* level of
    /// what remains, where [`StorePath::name_under`] is the first.
    pub fn entry_name(&self, key: &StorePath) -> Option<String> {
        key.strip_prefix(self)
            .and_then(|rest| rest.name().map(|name| name.into_owned()))
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
            segments: Segments::Deferred,
            joined: Joined::Owned(Arc::from(joined)),
        })
    }
}

/// Where a key read back from a store sits relative to the prefix it was
/// scanned from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Level<'a> {
    /// The key is the prefix itself.
    Prefix,

    /// The key is exactly one level below, named here.
    Entry(Cow<'a, str>),

    /// The key is more than one level below. The name is the level directly
    /// under the prefix; the rest of the key is inside it.
    Deeper(Cow<'a, str>),

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
/// [`Level::Entry`] and [`Level::Deeper`] are separate because a map owns the
/// level below it and nothing further: a key two levels down is somebody else's
/// and reading it as an entry hands back the wrong value under the wrong name.
///
/// The key is still checked, because a key this library did not write has to
/// be refused where it is read rather than believed.
pub fn level_under<'a>(key: &'a str, prefix: &StorePath) -> Result<Level<'a>, StorePathError> {
    validate_joined(key)?;
    Ok(level_below(key, prefix.as_str(), prefix.is_root()))
}

/// The level after `head` in `whole`, both joined.
fn level_below<'a>(whole: &'a str, head: &str, head_is_root: bool) -> Level<'a> {
    let rest = if head_is_root {
        whole
    } else if whole == head {
        return Level::Prefix;
    } else {
        match whole
            .strip_prefix(head)
            .and_then(|rest| rest.strip_prefix(SEPARATOR))
        {
            Some(rest) => rest,
            None => return Level::Outside,
        }
    };

    if rest.is_empty() {
        return Level::Prefix;
    }

    let mut escaped = false;
    for (at, ch) in rest.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            ESCAPE => escaped = true,
            SEPARATOR => return Level::Deeper(unescape(&rest[..at])),
            _ => {}
        }
    }

    Level::Entry(unescape(rest))
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

/// Orders two names the way the store orders the keys they become.
///
/// A flat engine sorts by the joined key, where a name is escaped, and escaping
/// does not preserve order: `.` and `\` both become a pair starting with `\`,
/// while everything between them keeps its own byte. So `"a.b"` sorts before
/// `"aAb"` by name and after it by key, and anything listing names in the
/// store's order has to compare them as the store will.
///
/// Compared without writing either escaped form out.
pub fn cmp_names(a: &str, b: &str) -> Ordering {
    escaped(a).cmp(escaped(b))
}

/// A name as it appears inside a joined path, with the separator and the
/// escape doubled. Borrowed unless the name holds one of them.
///
/// Two escaped names compare as [`cmp_names`] compares the names they came
/// from, so a collection ordered by this is ordered the way a store lists the
/// keys its entries become.
pub fn escape_name(name: &str) -> Cow<'_, str> {
    if !name.contains([SEPARATOR, ESCAPE]) {
        return Cow::Borrowed(name);
    }
    Cow::Owned(escaped(name).collect())
}

fn escaped(name: &str) -> impl Iterator<Item = char> + '_ {
    name.chars().flat_map(|ch| {
        let lead = (ch == SEPARATOR || ch == ESCAPE).then_some(ESCAPE);
        lead.into_iter().chain(std::iter::once(ch))
    })
}

fn join(segments: &[Arc<str>]) -> Arc<str> {
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

    Arc::from(out.as_str())
}

/// Equal paths are the ones that address the same place, and the joined form
/// says which that is: the escaping is injective, so two paths join to one
/// string exactly when they hold the same levels. Comparing the string rather
/// than walking the levels is the same question asked of the cheaper form, and
/// it agrees with [`Ord`] by construction.
impl PartialEq for StorePath {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for StorePath {}

/// The order a store lists in: the joined form, byte by byte.
///
/// The flat engines range over that string, so anything sorting paths for
/// itself has to agree with them or a listing changes order by engine.
impl Ord for StorePath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for StorePath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for StorePath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

/// A path and everything under it, in the key space a flat engine ranges over.
pub struct Subtree<'a> {
    prefix: &'a str,
    /// The prefix and a separator; `None` at the root.
    bound: Option<String>,
}

impl<'a> Subtree<'a> {
    /// Whether `key` names this path or something under it.
    pub fn contains(&self, key: &str) -> bool {
        match &self.bound {
            Some(bound) => key == self.prefix || key.starts_with(bound.as_str()),
            None => true,
        }
    }

    /// The half-open range the subtree occupies. Neither end is exact - the
    /// caller applies [`Subtree::contains`] to what it hands back - and the
    /// root has no top.
    pub fn range(&self) -> (&str, Option<String>) {
        if self.bound.is_none() {
            return ("", None);
        }

        let mut high = String::with_capacity(self.prefix.len() + 1);
        high.push_str(self.prefix);
        high.push((SEPARATOR as u8 + 1) as char);
        (self.prefix, Some(high))
    }

    /// Where the children begin; `None` at the root.
    pub fn child_bound(&self) -> Option<&str> {
        self.bound.as_deref()
    }

    /// The joined prefix.
    pub fn prefix(&self) -> &str {
        self.prefix
    }
}

impl fmt::Display for Subtree<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.range() {
            (low, Some(high)) => write!(f, "[{low}, {high})"),
            (low, None) => write!(f, "[{low}, and no upper bound)"),
        }
    }
}

/// Lets a collection keyed by `StorePath` be probed with the joined string.
/// Sound because `Hash`, `Eq` and `Ord` all delegate to [`StorePath::as_str`].
impl Borrow<str> for StorePath {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StorePath({:?})", self.as_str())
    }
}

impl fmt::Display for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for StorePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The same question asked of the levels instead of the joined form: two
    /// subtrees meet exactly when one level list starts the other.
    fn shares_a_key_by_levels(a: &StorePath, b: &StorePath) -> bool {
        let a: Vec<_> = a.segments().collect();
        let b: Vec<_> = b.segments().collect();
        let common = a.len().min(b.len());
        a[..common] == b[..common]
    }

    /// Two subtrees are nested or apart, and the string form is not the test.
    #[test]
    fn two_subtrees_overlap_exactly_when_one_holds_the_other() {
        let cases: &[(&[&str], &[&str], bool, &str)] = &[
            (&["ui"], &["ui"], true, "the same path"),
            (&["ui"], &["ui", "theme"], true, "a child"),
            (&["ui"], &["ui", "a", "b"], true, "a grandchild - a subtree is not one level"),
            (&["ui", "theme"], &["ui", "width"], false, "siblings"),
            (&["ui"], &["net"], false, "strangers"),
            (&["ui"], &["uix"], false, "a string prefix is not a level prefix"),
            (&["ui"], &["uix", "width"], false, "and neither is its subtree"),
            (&["ui"], &["ui!x"], false, "`!` sorts below the separator, and is still not under `ui`"),
            (&["ui.theme"], &["ui"], false, "one level literally named `ui.theme`, escaped as `ui\\.theme`"),
            (&["ui.theme"], &["ui", "theme"], false, "which is a different place from two levels"),
            (&[], &["ui", "theme"], true, "the root holds everything"),
        ];

        for (a, b, want, why) in cases {
            let (a, b) = (StorePath::from_segments(*a), StorePath::from_segments(*b));
            assert_eq!(a.overlaps(&b), *want, "{why}: {a} vs {b}");
            assert_eq!(b.overlaps(&a), *want, "{why}, the other way round: {b} vs {a}");
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
        /// `Subtree::contains` walks the joined string; the levels are a list.
        /// The two have to answer the same question, and this is the pair that
        /// escaping is between - a name carrying a separator is one level and
        /// two runs of the joined form.
        ///
        /// It also pins the shape: there is no third case. Whatever the two
        /// paths, they are nested or apart, so a key in both forces one to
        /// start the other.
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
                prop_assert!(a.subtree().contains(deeper.as_str()));
                prop_assert!(b.subtree().contains(deeper.as_str()));
            }
        }

        /// Two paths drawn independently almost never meet, so the true branch
        /// above is barely exercised. These two build the answer in.
        #[test]
        fn a_path_meets_everything_grown_from_it(
            head in path_strategy(),
            tail in prop::collection::vec(segment_strategy(), 0..4),
        ) {
            let a = StorePath::from_segments(&head);
            let b = StorePath::from_segments(head.iter().chain(&tail));

            prop_assert!(a.overlaps(&b), "{} does not hold {}", a, b);
            prop_assert!(a.subtree().contains(b.as_str()), "{} vs {}", a, b);
        }

        /// And the sharp case: one level apart under a shared ancestor. This is
        /// where a name carrying a separator would leak across if the joined
        /// form were read as characters rather than as levels.
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

        /// The point of the type, at any depth: a name is whatever the caller
        /// passed, and a separator inside it stays a character. The levels come
        /// back exactly as they went in, never more of them, and taking the
        /// names apart at the separator never lands on the same path - when it
        /// lands on one at all, since a name of nothing but separators splits
        /// into levels with no names.
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
        /// The joined form is the only thing a flat engine keeps, so the
        /// segments have to be recoverable from it - whatever the names hold.
        ///
        /// Follows from the two below: a key round trips, and the join is
        /// injective, so the levels recovered from `join(s)` can only be `s`.
        /// Kept because it fails saying that directly rather than leaving the
        /// inference to the reader.
        #[test]
        fn the_joined_form_round_trips(segments in path_strategy()) {
            let path = StorePath::from_segments(&segments);
            prop_assert_eq!(StorePath::parse_joined(path.as_str()).unwrap(), path);
        }

        /// The property the whole design rests on: two different sets of levels
        /// never land on the same key. Without it a name holding a separator
        /// could collide with a nesting a caller meant.
        #[test]
        fn different_levels_never_join_to_one_key(a in path_strategy(), b in path_strategy()) {
            let pa = StorePath::from_segments(&a);
            let pb = StorePath::from_segments(&b);

            prop_assert_eq!(a == b, pa.as_str() == pb.as_str());
        }

        /// Prefix matching is over levels, and stripping one is its inverse.
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

        /// One level at a time or all at once is the same path, and joining a
        /// single-level path is what pushing means. Equality is over the
        /// levels, so how a path was built never shows.
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

        /// Whatever spells the levels, the path is the same one.
        #[test]
        fn any_spelling_of_the_levels_gives_the_same_path(segments in path_strategy()) {
            let built = StorePath::from_segments(&segments);
            let refs: Vec<&str> = segments.iter().map(String::as_str).collect();

            prop_assert_eq!(refs.as_slice().into_store_path().unwrap(), built.clone());
            prop_assert_eq!(segments.clone().into_store_path().unwrap(), built.clone());
            prop_assert_eq!((&built).into_store_path().unwrap(), built.clone());
            prop_assert_eq!(built.clone().into_store_path().unwrap(), built);
        }

        /// The root is under everything, so stripping it is the identity.
        #[test]
        fn every_path_is_under_the_root(segments in path_strategy()) {
            let path = StorePath::from_segments(&segments);
            let root = StorePath::root();

            prop_assert!(path.starts_with(&root));
            prop_assert_eq!(path.strip_prefix(&root).unwrap(), path.clone());
            prop_assert!(!root.starts_with(&path));
        }

        /// A level with no name would join to the same string as the root, so
        /// it is not a path at all - wherever in the list it turns up.
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

        /// Any name is a name, however many separators it holds: building
        /// always succeeds, and however many of them the names contain, exactly
        /// one per gap between levels survives unescaped in the joined form.
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

        /// Growing a name always leaves the joined form a string prefix of the
        /// longer one, and the answer over levels is always still no. Reading
        /// that string prefix as a path prefix is what makes an unrelated
        /// subtree get scanned, or deleted.
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

        /// And the two answers never disagree: stripping succeeds exactly when
        /// the prefix is one, so nothing that is not under a prefix can be
        /// mistaken for something that is.
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

        /// The other half of the round trip, over strings this library did not
        /// write. A key that parses at all has to join back to the key it came
        /// from, because a flat engine addresses a value by that string: where
        /// two keys parse to one path, one of the two values is unreachable and
        /// the next write over that path destroys the other.
        #[test]
        fn a_key_that_parses_joins_back_to_itself(key in key_strategy()) {
            if let Ok(path) = StorePath::parse_joined(&key) {
                prop_assert_eq!(path.as_str(), key.as_str());
            }
        }

        /// A key holding a level with no name is refused, wherever the gap
        /// falls and whatever surrounds it.
        ///
        /// Such keys are already in stores - `.` was the sentinel for the whole
        /// document, and a trailing separator is what joining an empty key used
        /// to write. Reading one leniently would give the root a second name,
        /// or two keys one path.
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


        /// What a flat engine has to do with `as_str`, in both directions: a key
        /// is strictly under a path exactly when it starts with that path's key
        /// and a separator. The forward half is the escaping; the backward half
        /// is what stops a scan for `ui` reaching `uix.width`, and it is the
        /// only thing making the joined form safe to range over.
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

        /// Nothing that skips the check can make a path the check would refuse:
        /// whatever `join`, `push`, `parent`, `strip_prefix` and `parse_joined`
        /// hand back is rebuildable from its own levels, unchanged.
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

        /// `cmp_names` answers what comparing the joined keys answers, without
        /// writing them out. It is a second encoding of the escaping rule -
        /// `join` allocates, so a comparator cannot call it - and this is what
        /// keeps the two in step.
        #[test]
        fn comparing_names_answers_what_comparing_their_keys_answers(
            a in segment_strategy(),
            b in segment_strategy()
        ) {
            let under = StorePath::segment("m");
            let ka = under.push(&a);
            let kb = under.push(&b);

            prop_assert_eq!(
                cmp_names(&a, &b),
                ka.as_str().cmp(kb.as_str()),
                "names {:?} and {:?} became keys {:?} and {:?}",
                a, b, ka.as_str(), kb.as_str()
            );
        }

        /// Equality, hashing and the key never disagree, over paths built every
        /// way there is - including the root and the paths the derived calls
        /// hand back, which the other properties never reach.
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

        /// The const check and the join it checks are two writings of one rule,
        /// and they cannot be reduced to one: const evaluation has no
        /// allocator, so the verifier cannot call the join. This is the only
        /// thing keeping them in step, and without it they drift silently -
        /// which is how a second path parser went unnoticed once already.
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

    /// A golden for one decision, not for a rule: that separators inside a name
    /// are escaped is a property and covered by one, and that two different sets
    /// of levels never share a key is another. All this pins is which character
    /// does the escaping - changing it silently renames every key in every file
    /// already written.
    #[test]
    fn the_encoding_is_a_backslash_before_the_separator() {
        assert_eq!(StorePath::segment("dark.mode").as_str(), "dark\\.mode");
        assert_eq!(
            StorePath::from_segments(["dark", "mode"]).as_str(),
            "dark.mode"
        );
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

    /// The two smallest keys that are not the joined form of anything: an
    /// escape before an ordinary character, and an escape at the end. Both are
    /// read as if the escape were not there, so each is a second key for a path
    /// that already has one.
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

    /// The boundary that makes a flat scan safe cannot be spelled at the root.
    /// Every path is under the root, and no key can begin with a separator - a
    /// name that begins with one escapes it - so an engine that derives its
    /// scan bound the same way at every depth scans nothing at the top.
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

    /// A node's own key is not under its own boundary, so the string test and
    /// the level test disagree on exactly one path: the prefix itself. An
    /// engine that deletes a subtree by that boundary leaves the value stored at
    /// the node behind, and one that scans by it never lists that value.
    #[test]
    fn a_subtree_boundary_does_not_cover_the_node_itself() {
        let node = StorePath::segment("ui");
        let boundary = format!("{}{}", node.as_str(), SEPARATOR);

        assert!(node.push("width").as_str().starts_with(&boundary));
        assert!(!node.as_str().starts_with(&boundary));
        assert!(node.starts_with(&node));
    }

    /// Escaping covers the separator and the escape and nothing else, so a key
    /// carries whatever else a name held. The sqlite engine builds its scan out
    /// of `key GLOB prefix*`, where every one of these means something other
    /// than itself.
    #[test]
    fn a_key_carries_what_a_glob_pattern_reads() {
        let path = StorePath::segment("a*b[c]?\u{0}d");

        assert_eq!(path.as_str(), "a*b[c]?\u{0}d");
    }

    /// A path hashes as its key, because equality and order are that key too.
    ///
    /// All three answer from the joined form, which the escaping makes
    /// injective: two paths join to one string exactly when they hold the same
    /// levels. Agreeing on one form is what would make `Borrow<str>` sound, so
    /// a map keyed by paths could be probed with the string a flat engine
    /// already holds, instead of building a path for every lookup.
    #[test]
    fn a_path_hashes_like_its_key() {
        let path = StorePath::segment("ui");

        assert_eq!(path.as_str(), "ui");
        assert_eq!(hash_of(&path), hash_of(&"ui"));
    }

    /// A name is bytes, so two spellings a person and most editors read as one
    /// name are two levels with two keys. Normalising a file on save moves
    /// every value under such a name to a path nothing looks up.
    #[test]
    fn two_spellings_of_one_name_are_two_paths() {
        let precomposed = StorePath::segment("caf\u{e9}");
        let decomposed = StorePath::segment("cafe\u{301}");

        assert_ne!(precomposed, decomposed);
        assert_ne!(precomposed.as_str(), decomposed.as_str());
        assert_eq!(precomposed.to_string().chars().count(), 4);
        assert_eq!(decomposed.to_string().chars().count(), 5);
    }

    /// A document engine walks levels as `&[&str]`; the segments are
    /// `&[Arc<str>]` and nothing converts between them, so every get, set and
    /// delete allocates a vector to pass the path it was given.
    /// The document engines walk a level at a time, and now get `&str` without
    /// anything in between.
    #[test]
    fn walking_a_document_borrows_each_level() {
        let path = StorePath::from_segments(["ui", "window", "width"]);

        assert_eq!(
            path.segments().collect::<Vec<_>>(),
            ["ui", "window", "width"]
        );
        assert_eq!(path.segment_at(1).as_deref(), Some("window"));
        assert_eq!(path.segment_at(3), None);
    }

    /// A path the compiler knows costs nothing to build, and the two halves it
    /// is handed are checked against each other where they are written.
    #[test]
    fn a_static_path_is_the_same_path() {
        static UI_WIDTH: StorePath = StorePath::from_static(&["ui", "width"], "ui.width");

        assert_eq!(UI_WIDTH, StorePath::from_segments(["ui", "width"]));
        assert_eq!(UI_WIDTH.as_str(), "ui.width");
        assert_eq!(UI_WIDTH.name().as_deref(), Some("width"));
        assert!(UI_WIDTH.starts_with(&StorePath::from_static(&["ui"], "ui")));
    }

    /// The check is the join, so a name holding the separator or the escape
    /// still writes a static path - it only has to be spelled escaped, the way
    /// `as_str` would write it. The pairs that do not agree are refused during
    /// const evaluation, which is a compile error; `tests/fails` pins those.
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
