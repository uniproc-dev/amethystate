use amethystate::amethystate;
use amethystate::reactive::map::ReactiveMap;

type Sessions = ReactiveMap<String, String>;

#[amethystate(prefix = "aliased")]
pub struct State {
    #[amestate(default = {})]
    pub sessions: Sessions,
}

fn main() {}
