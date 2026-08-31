use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveScope, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;
use std::sync::{Arc, Mutex};

#[amethystate(prefix = "net")]
pub struct ConnectionState {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,
}

fn open(
    backend: Backend,
    tag: &str,
) -> Result<(ConnectionState, TempPath), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    Ok((ConnectionState::new_with(&store)?, path))
}

#[backends(all)]
fn a_subscription_lasts_as_long_as_its_handle(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_subs_raii")?;
    let heard = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&heard);

    //@show subscribing, and letting the subscription go
    let sub = state.port().subscribe(move |port| {
        seen.lock().unwrap().push(*port);
    });

    state.port().set(9090)?;
    assert_eq!(*heard.lock().unwrap(), [9090]);

    drop(sub);

    state.port().set(1234)?;
    assert_eq!(*heard.lock().unwrap(), [9090]);

    let ignored = Arc::clone(&heard);
    let _ = state.port().subscribe(move |port| {
        ignored.lock().unwrap().push(*port);
    });

    state.port().set(4321)?;
    assert_eq!(*heard.lock().unwrap(), [9090]);
    //@show-end

    Ok(())
}

#[backends(all)]
fn a_scope_holds_several_at_once(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_subs_scope")?;

    //@show keeping several subscriptions in one place
    let mut scope = ReactiveScope::new();

    state
        .port()
        .subscribe(|port| println!("port {port}"))
        .watch(&mut scope);
    state
        .host()
        .subscribe(|host| println!("host {host}"))
        .watch(&mut scope);

    scope.clear();
    //@show-end

    Ok(())
}

#[backends(all)]
fn ignoring_your_own_writes(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_subs_external")?;
    let heard = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&heard);

    //@show hearing only about somebody else's writes
    let watcher = state.port().fork();

    let _sub = state
        .port()
        .subscription_with()
        .external()
        .register(move |port| {
            seen.lock().unwrap().push(*port);
        });

    state.port().set(8080)?;
    watcher.set(9090)?;

    assert_eq!(*heard.lock().unwrap(), [9090]);
    //@show-end

    Ok(())
}

#[backends(all)]
fn a_clone_is_the_same_actor_and_a_fork_is_not(
    backend: Backend,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_subs_identity")?;

    let heard = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&heard);

    //@show the same actor, and a different one
    let port = state.port();
    let same = port.clone();
    let other = port.fork();

    let _sub = port
        .subscription_with()
        .external()
        .register(move |value| seen.lock().unwrap().push(*value));

    same.set(1111)?;
    other.set(2222)?;

    assert_eq!(*heard.lock().unwrap(), [2222]);

    assert_eq!(port.instance_id(), same.instance_id());
    assert_ne!(port.instance_id(), other.instance_id());
    //@show-end

    Ok(())
}

#[backends(all)]
fn who_made_the_change(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (state, _path) = open(backend, "book_subs_source")?;
    let heard = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&heard);

    //@show asking who made the change
    let _sub = state
        .port()
        .subscription_with()
        .register_with_source(move |port, who| {
            seen.lock().unwrap().push((*port, who));
        });

    state.port().set(9090)?;

    let (port, who) = heard.lock().unwrap()[0];
    assert_eq!(port, 9090);
    assert_eq!(who, Some(state.port().instance_id()));
    //@show-end

    Ok(())
}
