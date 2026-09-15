#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyDirection {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlintProperty {
    pub name: String,
    pub rust_name: String,
    pub ty: SlintType,
    pub direction: PropertyDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlintType {
    String,
    Bool,
    Int,
    Float,
    Color,
    Brush,
    Image,
    Duration,
    Length,
    Named(String),
}

impl SlintType {
    pub fn has_default_into(&self) -> bool {
        matches!(
            self,
            SlintType::String | SlintType::Bool | SlintType::Int | SlintType::Float
        )
    }

    pub fn slint_rust_type(&self) -> &str {
        match self {
            SlintType::String => "slint::SharedString",
            SlintType::Bool => "bool",
            SlintType::Int => "i32",
            SlintType::Float => "f32",
            SlintType::Color => "slint::Color",
            SlintType::Brush => "slint::Brush",
            SlintType::Image => "slint::Image",
            SlintType::Duration => "i64",
            SlintType::Length => "f32",
            SlintType::Named(n) => n.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlintComponent {
    pub name: String,
    pub properties: Vec<SlintProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlintGlobal {
    pub name: String,
    pub properties: Vec<SlintProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlintTypeDef {
    Struct(String),
    Enum(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlintFile {
    pub components: Vec<SlintComponent>,
    pub globals: Vec<SlintGlobal>,
    pub type_defs: Vec<SlintTypeDef>,
}
