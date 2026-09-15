use amethystate_core::test_utils::TempPath;

//@show the readme's first example
use amethystate::{StoreBuilder, amethystate};

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = StoreBuilder::new("./app").build()?;
    let state = NetworkState::new_with(&store)?;

    let _sub = state.port().subscribe(|port| println!("port → {port}"));

    state.port().set(9090)?;

    Ok(())
}
//@show-end

#[test]
fn the_readme_example_compiles_and_runs() {
    let dir = TempPath::new("book_readme");
    let home = std::env::current_dir().unwrap();

    std::env::set_current_dir(dir.path().parent().unwrap()).unwrap();
    let ran = main().map_err(|why| why.to_string());
    std::env::set_current_dir(home).unwrap();

    ran.unwrap();
}
