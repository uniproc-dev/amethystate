use amethystate::amethystate;

#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct ReactiveMap<K, V> {
    pub pairs: Vec<(K, V)>,
}

#[amethystate(prefix = "byname")]
pub struct State {
    pub sessions: ReactiveMap<String, String>,
}

fn main() {
    let _: fn(&State) -> amethystate::Field<ReactiveMap<String, String>> = State::sessions;
}
