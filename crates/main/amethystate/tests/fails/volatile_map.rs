use amethystate::ReactiveMap;
use amethystate_macros::amethystate;

#[amethystate(prefix = "editor")]
pub struct Editor {
    #[amestate(volatile)]
    pub open: ReactiveMap<String, String>,
}

fn main() {}
