use super::document::Navigable;
use amethystate_core::path::{SmolStr, Stored};
use indexmap::IndexMap;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::sync::Arc;

/// A level's children, in the order the file holds them.
pub type Fields = IndexMap<SmolStr, Node>;

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
    Map(Arc<Fields>),
}

impl Node {
    /// A level holding `fields`.
    pub fn map(fields: Fields) -> Self {
        Node::Map(Arc::new(fields))
    }

    /// The children, or `None` where this is a value.
    pub fn fields(&self) -> Option<&Fields> {
        match self {
            Node::Map(fields) => Some(fields),
            _ => None,
        }
    }

    /// The children to write into, cloning this level if it is shared.
    fn fields_mut(&mut self) -> Option<&mut Fields> {
        match self {
            Node::Map(fields) => Some(Arc::make_mut(fields)),
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
            Node::Map(held) => out.collect_map(held.iter().map(|(key, node)| (&**key, node))),
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
