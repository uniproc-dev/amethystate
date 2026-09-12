use super::document::Navigable;
use amethystate_core::path::{SmolStr, Stored};
use indexmap::IndexMap;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::sync::Arc;

/// A level's children, in the order the file holds them.
pub type Fields = IndexMap<SmolStr, Node>;

/// A level, and what everything under it hashes to once somebody has asked.
///
/// The hash is what makes a diff cost the size of the change: two levels that
/// hash alike hold the same subtree, so the walk stops there instead of
/// descending. Comparing them is two `u128`s where comparing the levels
/// themselves is the whole subtree.
///
/// Taken lazily rather than at parse, so a document nobody compares is never
/// hashed, and cleared in [`Node::fields_mut`] - the one place a mutation can
/// reach a level - so a level that was written to answers again rather than
/// answering stale.
#[derive(Clone)]
pub struct Branch {
    fields: Fields,
    hash: std::sync::OnceLock<u128>,
}

impl Branch {
    fn new(fields: Fields) -> Self {
        Self {
            fields,
            hash: std::sync::OnceLock::new(),
        }
    }

    fn hash(&self) -> u128 {
        *self.hash.get_or_init(|| {
            let mut held = xxhash_rust::xxh3::Xxh3::new();
            mix_fields(&mut held, &self.fields);
            held.digest128()
        })
    }
}

impl fmt::Debug for Branch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.fields, f)
    }
}

/// A value in a document, owned by the store rather than by a format's library.
///
/// Every level is shared behind an `Arc`, so copying a node copies a pointer
/// and a write clones only the levels it passes through. A version of a
/// document is therefore a node, and two versions share everything neither of
/// them changed.
#[derive(Clone, Debug)]
pub enum Node {
    Null,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Text(SmolStr),
    List(Arc<Vec<Node>>),
    Map(Arc<Branch>),
}

impl Node {
    /// A level holding `fields`.
    pub fn map(fields: Fields) -> Self {
        Node::Map(Arc::new(Branch::new(fields)))
    }

    /// The children, or `None` where this is a value.
    pub fn fields(&self) -> Option<&Fields> {
        match self {
            Node::Map(held) => Some(&held.fields),
            _ => None,
        }
    }

    /// The children to write into, cloning this level if it is shared.
    ///
    /// Also where the level forgets what it hashed to. This is the only way a
    /// mutation reaches a level - `get_child_mut`, `insert_child` and
    /// `remove_child` all come through here - so clearing it here is what makes
    /// the hash safe to trust anywhere else.
    fn fields_mut(&mut self) -> Option<&mut Fields> {
        match self {
            Node::Map(held) => {
                let held = Arc::make_mut(held);
                held.hash = std::sync::OnceLock::new();
                Some(&mut held.fields)
            }
            _ => None,
        }
    }
}

impl Navigable for Node {
    fn make_empty_map() -> Self {
        Node::map(Fields::new())
    }

    fn get_child(&self, key: Stored<'_>) -> Option<&Self> {
        self.fields()?.get(key.as_str())
    }

    fn get_child_mut(&mut self, key: Stored<'_>) -> Option<&mut Self> {
        self.fields_mut()?.get_mut(key.as_str())
    }

    fn is_map(&self) -> bool {
        matches!(self, Node::Map(_))
    }

    fn has_children(&self) -> bool {
        self.fields().is_some_and(|fields| !fields.is_empty())
    }

    fn insert_child(&mut self, key: Stored<'_>, val: Self) {
        if let Some(fields) = self.fields_mut() {
            fields.insert(SmolStr::new(key.as_str()), val);
        }
    }

    fn remove_child(&mut self, key: Stored<'_>) -> Option<Self> {
        self.fields_mut()?.shift_remove(key.as_str())
    }

    fn scan_children(&self) -> Vec<(SmolStr, Self)> {
        match self.fields() {
            Some(fields) => fields
                .iter()
                .map(|(key, node)| (key.clone(), node.clone()))
                .collect(),
            None => Vec::new(),
        }
    }

    fn child_names(&self) -> Vec<SmolStr> {
        match self.fields() {
            Some(fields) => fields.keys().cloned().collect(),
            None => Vec::new(),
        }
    }

    fn each_child(&self) -> impl Iterator<Item = (&str, &Self)> {
        self.fields()
            .into_iter()
            .flat_map(|fields| fields.iter().map(|(key, node)| (key.as_str(), node)))
    }

    /// Two levels answer from their hashes, everything else compares itself.
    ///
    /// A level is where the saving is - its hash stands for a whole subtree -
    /// and a leaf is where it would be a loss: hashing a number to avoid
    /// comparing a number is work for nothing.
    fn known_same(&self, other: &Self) -> bool {
        match (self, other) {
            (Node::Map(ours), Node::Map(theirs)) => {
                Arc::ptr_eq(ours, theirs) || ours.hash() == theirs.hash()
            }
            _ => self == other,
        }
    }
}

/// Written out rather than derived, for the one line a derive would not have:
/// two levels that are the same allocation are the same level, and saying so
/// stops the walk at the top of every subtree a write did not pass through.
///
/// That shortcut only fires between versions descended from one another - a
/// document read back from a file shares nothing with the one held in memory -
/// so it is worth what a snapshot is worth, not what a diff against the disk
/// is.
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // Ordered, where a map's own equality is not: the order is the
            // order the file holds and a render writes back, so two levels with
            // the same pairs in a different order are two different documents.
            (Node::Map(ours), Node::Map(theirs)) => {
                Arc::ptr_eq(ours, theirs)
                    || (ours.fields.len() == theirs.fields.len()
                        && ours.fields.iter().zip(theirs.fields.iter()).all(
                            |((ours, under), (theirs, beside))| ours == theirs && under == beside,
                        ))
            }
            (Node::List(ours), Node::List(theirs)) => Arc::ptr_eq(ours, theirs) || ours == theirs,
            (Node::Null, Node::Null) => true,
            (Node::Bool(ours), Node::Bool(theirs)) => ours == theirs,
            (Node::Int(ours), Node::Int(theirs)) => ours == theirs,
            (Node::Uint(ours), Node::Uint(theirs)) => ours == theirs,
            // By the bits, not by the number. `-0.0 == 0.0` is true and they
            // render differently, so equality by value would call a change no
            // change; and a `NaN` equals no number at all, itself included,
            // where the bytes of one are the bytes of one.
            (Node::Float(ours), Node::Float(theirs)) => ours.to_bits() == theirs.to_bits(),
            (Node::Text(ours), Node::Text(theirs)) => ours == theirs,
            _ => false,
        }
    }
}

/// What a subtree hashes to, for telling two readings apart without walking
/// both.
///
/// 128 bits because the way this fails is silence: two different subtrees
/// hashing alike is a change that is never reported, and that is worse than a
/// crash. At 128 the odds of one over a century of diffing a hundred thousand
/// keys a second are around 2^-80 - below a cosmic ray flipping the answer in
/// memory, which is the floor everything else here runs on. At 64 they are
/// about one in sixty-five thousand over the same century, which is not a
/// number to put behind "the diff does not lose events".
///
/// Two things keep a collision improbable rather than constructible. Each kind
/// of node mixes a tag of its own, so the string `"1"` and the number `1` do
/// not hash alike by simply being the same bytes. And a level mixes its names
/// in the order it holds them, because that order is what the file holds and
/// what a render writes back: two levels with the same pairs in a different
/// order are two different documents here.
pub fn subtree_hash(node: &Node) -> u128 {
    let mut held = xxhash_rust::xxh3::Xxh3::new();
    mix(&mut held, node);
    held.digest128()
}

fn mix(into: &mut xxhash_rust::xxh3::Xxh3, node: &Node) {
    match node {
        Node::Null => into.update(&[0]),
        Node::Bool(held) => {
            into.update(&[1]);
            into.update(&[u8::from(*held)]);
        }
        Node::Int(held) => {
            into.update(&[2]);
            into.update(&held.to_le_bytes());
        }
        Node::Uint(held) => {
            into.update(&[3]);
            into.update(&held.to_le_bytes());
        }
        Node::Float(held) => {
            into.update(&[4]);
            into.update(&held.to_le_bytes());
        }
        Node::Text(held) => {
            into.update(&[5]);
            into.update(&(held.len() as u64).to_le_bytes());
            into.update(held.as_bytes());
        }
        Node::List(held) => {
            into.update(&[6]);
            into.update(&(held.len() as u64).to_le_bytes());
            for item in held.iter() {
                mix(into, item);
            }
        }
        Node::Map(held) => {
            into.update(&[7]);
            mix_fields(into, &held.fields);
        }
    }
}

fn mix_fields(into: &mut xxhash_rust::xxh3::Xxh3, fields: &Fields) {
    into.update(&(fields.len() as u64).to_le_bytes());

    for (name, under) in fields.iter() {
        into.update(&(name.len() as u64).to_le_bytes());
        into.update(name.as_bytes());
        mix(into, under);
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        match self {
            Node::Null => out.serialize_unit(),
            Node::Bool(held) => out.serialize_bool(*held),
            Node::Int(held) => out.serialize_i64(*held),
            Node::Uint(held) => out.serialize_u64(*held),
            Node::Float(held) if held.is_finite() => out.serialize_f64(*held),
            Node::Float(_) => out.serialize_unit(),
            Node::Text(held) => out.serialize_str(held),
            Node::List(held) => out.collect_seq(held.iter()),
            Node::Map(held) => {
                out.collect_map(held.fields.iter().map(|(key, node)| (&**key, node)))
            }
        }
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        from.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value a document can hold")
    }

    fn visit_bool<E>(self, held: bool) -> Result<Node, E> {
        Ok(Node::Bool(held))
    }

    fn visit_i64<E>(self, held: i64) -> Result<Node, E> {
        Ok(Node::Int(held))
    }

    fn visit_u64<E>(self, held: u64) -> Result<Node, E> {
        match i64::try_from(held) {
            Ok(fits) => Ok(Node::Int(fits)),
            Err(_) => Ok(Node::Uint(held)),
        }
    }

    fn visit_f64<E>(self, held: f64) -> Result<Node, E> {
        Ok(Node::Float(held))
    }

    fn visit_str<E>(self, held: &str) -> Result<Node, E> {
        Ok(Node::Text(SmolStr::new(held)))
    }

    fn visit_string<E>(self, held: String) -> Result<Node, E> {
        Ok(Node::Text(SmolStr::new(held)))
    }

    fn visit_none<E>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, from: D) -> Result<Node, D::Error> {
        Node::deserialize(from)
    }

    fn visit_unit<E>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, from: D) -> Result<Node, D::Error> {
        Node::deserialize(from)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Node, A::Error> {
        let mut held = Vec::with_capacity(items.size_hint().unwrap_or(0));

        while let Some(item) = items.next_element()? {
            held.push(item);
        }

        Ok(Node::List(Arc::new(held)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Node, A::Error> {
        let mut held = Fields::with_capacity(entries.size_hint().unwrap_or(0));

        while let Some((key, node)) = entries.next_entry::<String, Node>()? {
            held.insert(SmolStr::new(key), node);
        }

        Ok(Node::map(held))
    }
}
