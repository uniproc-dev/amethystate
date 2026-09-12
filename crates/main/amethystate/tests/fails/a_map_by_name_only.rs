use amethystate::amethystate;

#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct ReactiveMap<K, V> {
    pub pairs: Vec<(K, V)>,
}

#[amethystate(prefix = "byname")]
pub struct State {
    #[amestate(default = {})]
    pub sessions: ReactiveMap<String, String>,
}

fn main() {}
