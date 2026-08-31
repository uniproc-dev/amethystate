use amethystate::observability;
use amethystate::store::builder::Backend;
use amethystate::{StoreBuilder, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use tracing_test::traced_test;

#[amethystate(prefix = "obs")]
pub struct ObsState {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "localhost".to_string())]
    pub host: String,
}

/// A field whose own name holds the separator keeps it.
///
/// The registry used to take the joined path as a string and recover the name
/// by splitting on the last separator, which reads straight past the escaping:
/// a field named `a.b` is stored as `obs.a\.b`, and the split called it `b`.
/// The path knows where its levels are, so it is asked.
#[test]
fn a_field_named_with_a_separator_is_registered_under_that_name() {
    use amethystate::uuid::Uuid;
    use amethystate_core::path::StorePath;

    let id = Uuid::new_v4();
    observability::register_instance(id, "SeparatorNamed");
    let path = StorePath::from_segments(["obs_sep", "a.b"]);
    observability::register_field::<u16>(&path, id);

    let meta = observability::resolve_field(path.as_str())
        .expect("the field must be in the schema registry under the key it was written at");

    assert_eq!(
        meta.field_name.as_ref(),
        "a.b",
        "the name was cut at the separator inside it"
    );
}

#[backends(all)]
fn instance_registered_on_new(backend: Backend) {
    let path = unique_path("obs_instance_reg");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let _state = ObsState::new_with(&store).unwrap();

    let port_meta = observability::resolve_field("obs.port")
        .expect("obs.port must be in schema registry after construction");

    assert!(
        port_meta.struct_type_name.contains("ObsState"),
        "struct_type_name should contain 'ObsState', got: {}",
        port_meta.struct_type_name
    );
}

#[backends(all)]
fn fields_registered_in_schema_registry(backend: Backend) {
    let path = unique_path("obs_schema_reg");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let _state = ObsState::new_with(&store).unwrap();

    let port_meta =
        observability::resolve_field("obs.port").expect("obs.port must be in schema registry");
    assert_eq!(port_meta.field_name.as_ref(), "port");
    assert!(
        port_meta.struct_type_name.contains("ObsState"),
        "struct_type_name should reference ObsState, got: {}",
        port_meta.struct_type_name
    );
    assert!(
        port_meta.value_type_name.contains("u16"),
        "value_type_name should be u16, got: {}",
        port_meta.value_type_name
    );

    let host_meta =
        observability::resolve_field("obs.host").expect("obs.host must be in schema registry");
    assert_eq!(host_meta.field_name.as_ref(), "host");
    assert!(host_meta.value_type_name.contains("String"));
}

#[backends(all)]
#[traced_test]
fn field_set_emits_trace(backend: Backend) {
    let path = unique_path("obs_write_trace");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = ObsState::new_with(&store).unwrap();

    state.port().set(9090).unwrap();

    assert!(logs_contain("field write"), "expected 'field write' trace");
    assert!(
        logs_contain("obs.port"),
        "expected path 'obs.port' in trace"
    );
}

#[backends(all)]
#[traced_test]
fn field_set_trace_contains_source_name(backend: Backend) {
    let path = unique_path("obs_source_name");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = ObsState::new_with(&store).unwrap();

    state.host().set("example.com".to_string()).unwrap();

    assert!(
        logs_contain("ObsState"),
        "expected struct name 'ObsState' in write trace"
    );
}

#[backends(all)]
#[traced_test]
fn subscription_fire_emits_trace_with_location(backend: Backend) {
    let path = unique_path("obs_sub_trace");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = ObsState::new_with(&store).unwrap();

    let _sub = state.port().subscribe(|_| {});
    state.port().set(1234).unwrap();

    assert!(
        logs_contain("signal emit"),
        "expected 'signal emit' trace on subscription fire"
    );
    assert!(
        logs_contain("observability_tracing.rs"),
        "expected call-site file name in subscription trace"
    );
}

/// A subscription built through [`Watch`] carries the same location a direct
/// `subscribe` does. The builder is three calls deep, and each of them would
/// otherwise put its own line in this library where the caller's belongs.
#[backends(all)]
#[traced_test]
fn a_built_subscription_traces_the_call_site_and_not_the_builder(backend: Backend) {
    let path = unique_path("obs_watch_trace");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = ObsState::new_with(&store).unwrap();

    let _sub = state.port().subscription_with().register(|_| {});
    state.port().set(4321).unwrap();

    assert!(
        logs_contain("signal emit"),
        "the subscription never fired, so there is no location to check"
    );
    assert!(
        logs_contain("observability_tracing.rs"),
        "expected the call site in the trace"
    );
    assert!(
        !logs_contain("watch.rs"),
        "the trace blamed the builder instead of the caller"
    );
}

#[backends(all)]
#[traced_test]
fn named_subscription_appears_in_trace(backend: Backend) {
    let path = unique_path("obs_named_sub");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = ObsState::new_with(&store).unwrap();

    let _sub = state.port().subscribe(|_| {}).named("PortWatcher");
    state.port().set(5555).unwrap();

    assert!(
        logs_contain("PortWatcher"),
        "expected named subscription label 'PortWatcher' in trace"
    );
}

#[backends(all)]
#[traced_test]
fn forked_write_traces_with_source(backend: Backend) {
    let path = unique_path("obs_fork_trace");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = ObsState::new_with(&store).unwrap();
    let fork = state.fork();

    fork.port().set(7777).unwrap();

    assert!(
        logs_contain("field write"),
        "expected trace from fork write"
    );
    assert!(logs_contain("obs.port"));
}
