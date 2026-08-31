use amethystate_macros::amethystate;

#[amethystate(prefix = "tree")]
pub struct Node {
    #[amestate(default = 0u32)]
    pub depth: u32,

    #[amestate(nested)]
    pub child: Node,
}

fn main() {}
