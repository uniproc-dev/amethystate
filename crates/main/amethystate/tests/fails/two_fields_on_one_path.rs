use amethystate::amethystate;

#[amethystate(prefix = "dup")]
pub struct Two {
    #[amestate(path = "same", default = 1u32)]
    pub first: u32,

    #[amestate(path = "same", default = 2u32)]
    pub second: u32,
}

#[amethystate(prefix = "renamed", rename_all = "lowercase")]
pub struct Folded {
    #[amestate(default = 1u32)]
    pub theme: u32,

    #[amestate(path = "theme", default = 2u32)]
    pub also_theme: u32,
}

#[amethystate(prefix = "overlap")]
pub struct UnderAMap {
    #[amestate(default = {})]
    pub bag: amethystate::ReactiveMap<String, u32>,

    #[amestate(path = "bag.sneaky", default = 99u32)]
    pub sneaky: u32,
}

#[amethystate]
pub struct Inner {
    #[amestate(default = 1u32)]
    pub b: u32,
}

#[amethystate(prefix = "collide")]
pub struct DottedAgainstNested {
    #[amestate(nested)]
    pub a: Inner,

    #[amestate(path = "a.b", default = 2u32)]
    pub dotted: u32,
}

#[amethystate(prefix = "onthenode")]
pub struct LeafOnANodesGround {
    #[amestate(nested)]
    pub ui: Inner,

    #[amestate(path = "ui", default = 3u32)]
    pub marker: u32,
}

fn main() {}
