use amethystate::StoreBuilder;

fn main() {
    let _ = StoreBuilder::new("./app")
        .migrations(|m| {
            m.for_prefix("net").step(1, "never runs", |_| Ok(()));
        })
        .build();
}
