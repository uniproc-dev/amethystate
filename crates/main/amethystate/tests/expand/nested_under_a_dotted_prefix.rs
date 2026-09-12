use amethystate_macros::amethystate;

#[amethystate]
pub struct ConnectionPool {
    #[amestate(default = 10)]
    pub max_connections: u32,

    #[amestate(default = 30)]
    pub timeout_secs: u32,
}

#[amethystate(prefix = "sys.database")]
pub struct DatabaseState {
    #[amestate(nested)]
    pub pool: ConnectionPool,
}

fn main() {}
