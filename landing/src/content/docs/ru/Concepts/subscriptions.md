---
title: Подписки
sidebar:
  order: 17
---

Механизм подписок у всех реактивных примитивов один и тот же, поэтому
написанное здесь одинаково верно для
[`Field`](/amethystate/ru/primitives/field/),
[`ReactiveCell`](/amethystate/ru/primitives/reactive-cell/) и
[`ReactiveMap`](/amethystate/ru/primitives/reactive-map/).

## Подписка живёт, пока жив guard

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
```
<!-- /shown -->

`subscribe` отдаёт guard, и колбэк живёт ровно столько, сколько он. Отдельного
`unsubscribe` нет: регистрация снимается, когда guard дропают.

## Держать guard — забота вызывающего

Отсюда ловушка. Guard, присвоенный `_`, дропается в конце того же выражения, и
колбэк не срабатывает ни разу:

<!-- shown: a subscription nobody held on to -->
```rust
let ignored = Arc::clone(&heard);
let _ = state.port().subscribe(move |port| {
    ignored.lock().unwrap().push(*port);
});

state.port().set(4321)?;
assert_eq!(*heard.lock().unwrap(), [9090]);
```
<!-- /shown -->

`heard` так и остался с `[9090]` — от первой подписки. Поэтому каждый пример
здесь связывает guard с именем, а класть его стоит рядом с тем, куда пишет
колбэк, чтобы оба умерли вместе.

### Несколько сразу

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

`ReactiveScope` — один владелец сразу для нескольких guard. `clear` дропает их
все, дроп самой области делает то же.

## Когда обычной подписки недостаточно

`subscribe` закрывает обычный случай. Всё, что дальше, идёт через
`subscription_with()`, и звенья у него складываются в любом порядке:

| звено | что делает |
| --- | --- |
| `.external()` | пропускает изменения, сделанные этим хендлом |
| `.key(k)` | на карте — сужает до одной записи |
| `.register(f)` | завершает, отдавая guard |
| `.register_with_source(f)` | то же, но ещё и с тем, кто сделал изменение |
| `.stream()` | завершает как `Stream`, а не как колбэк |

## Как забрать изменения в собственный цикл

Колбэк обязан быть `Send + Sync`: правку, сделанную в файле мимо процесса,
приносит поток наблюдателя. Это отсекает состояние на `Rc` и почти все хендлы
контекста GUI.

`.stream()` завершает подписку `Stream`'ом. Границу потока переходит значение и
больше ничего, поэтому всё, что вы с ним делаете, происходит на том потоке,
который крутит цикл:

```rust
let mut ports = state.port().subscription_with().stream();

while let Some(port) = ports.next().await {
    label.set_text(&port.to_string());
}
```

`Stream` отдаёт каждое изменение и ничего не склеивает: это последовательность,
а склеивать их или нет — решаете вы, дальше по цепочке. Дроп `Stream`
заканчивает подписку.

## Чья это была запись

Каждая запись несёт id того хендла, который её сделал, и `.external()` этим и
пользуется:

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

Фоновый поток пишет, интерфейс отзывается — вот обычная расстановка: поток
держит форк, интерфейс подписывается через `.external()` и на собственные
записи не отзывается.

У правки, сделанной мимо процесса — файл правили руками, — id нет вовсе. Значит,
ничьей собственной записью она не считается и доходит в том числе до подписчиков
`external`.

### `clone` и `fork`

Оба дают ещё один хендл на то же значение, а расходятся в одном: чьими считать
их записи.

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

`clone` оставляет id прежним: оригинал и клон — один актор, и через `external`
записи друг друга они не слышат. `fork` берёт новый id: это уже двое, и каждый
слышит другого.

Этот id и есть `instance_id()` — он есть и у `Field`, и у `ReactiveMap`.
Спрашивайте его у хендла, когда id нужно подержать в руках, а не отфильтровать
по нему: назвать актора в логе или сравнить с тем id, с которым приходит
изменение, — про это следующий раздел.

### Автор приходит вместе со значением

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

`register_with_source` кладёт колбэку id рядом со значением — чтобы решать по
каждому изменению, а не отсеивать оптом.

## `external` на `ReactiveMap`

`.external()` фильтрует один `Update`: `Insert`, `Remove` и `Clear` доходят до
всех, включая того, кто их и вызвал. Почему так и что из этого следует для
`insert` — на странице
[ReactiveMap](/amethystate/ru/primitives/reactive-map/#что-фильтрует-external).
