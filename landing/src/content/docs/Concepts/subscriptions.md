---
title: Subscriptions
sidebar:
  order: 17
---

Every reactive primitive is watched the same way, so what is here is true of a
[field](/amethystate/primitives/field/), a
[cell](/amethystate/primitives/reactive-cell/) and a
[map](/amethystate/primitives/reactive-map/) alike.

## The handle is the subscription

<!-- shown: subscribing, and letting the subscription go -->
```rust
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
```
<!-- /shown -->

`subscribe` returns a handle and the callback lives exactly as long as it does.
Dropping the handle unregisters; a handle assigned to `_` is dropped at the end
of the statement and never fires at all, which is why every example here binds
it to a name.

Keeping one is the caller's job - store it beside whatever the callback writes
into, so the two die together.

### Several at once

<!-- shown: keeping several subscriptions in one place -->
```rust
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
```
<!-- /shown -->

`ReactiveScope` is one owner for many handles. `clear` drops them all; dropping
the scope does the same.

## Configuring one

`subscribe` covers the common case. Anything past it goes through
`subscription_with()`, whose links compose in any order:

| link | what it does |
| --- | --- |
| `.external()` | skip changes this handle made |
| `.key(k)` | on a map, narrow to one entry |
| `.register(f)` | finish, returning the handle |
| `.register_with_source(f)` | the same, with who made the change |
| `.stream()` | finish as a `Stream` rather than a callback |

## Taking the changes into a loop of your own

A callback must be `Send + Sync`, because a change made to the file outside the
process is delivered from a watcher thread. That rules out `Rc` state and most
GUI context handles.

`.stream()` finishes the subscription as a `Stream` instead. The value crosses
the thread boundary and nothing else does, so what you do with it runs on the
thread that drives the loop:

```rust
let mut ports = state.port().subscription_with().stream();

while let Some(port) = ports.next().await {
    label.set_text(&port.to_string());
}
```

A stream yields every change rather than coalescing - it is a sequence, and
coalescing downstream is your choice. Dropping it ends the subscription.

## Whose write was it

Every write carries the id of the handle that made it, and `.external()` is the
filter that uses it:

<!-- shown: hearing only about somebody else's writes -->
```rust
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
```
<!-- /shown -->

A background thread writing while the UI reacts is the usual shape: the thread
holds a fork, the UI subscribes `.external()`, and the UI does not redraw on its
own writes.

A change made outside the process - the file edited by hand - has no id at all,
so it is nobody's own write and reaches `external` subscribers too.

### `clone` and `fork`

Both give another handle onto the same value. They differ in one thing: whose
writes they count as.

<!-- shown: the same actor, and a different one -->
```rust
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
```
<!-- /shown -->

`clone` keeps the id, so the original and the clone are one actor and neither
hears the other's writes through `external`. `fork` takes a new id, so the two
are separate actors and each hears the other.

### Asking directly

<!-- shown: asking who made the change -->
```rust
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
```
<!-- /shown -->

`register_with_source` hands the callback the id beside the value, for deciding
per change rather than filtering wholesale.

## On a map

`.external()` filters `Update` and nothing else - `Insert`, `Remove` and `Clear`
reach everyone including whoever caused them. The reasoning, and what that
implies for `insert`, is on
[ReactiveMap](/amethystate/primitives/reactive-map/#what-external-filters).
