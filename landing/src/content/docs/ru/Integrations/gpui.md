---
title: GPUI
---


GPUI использует модель сущностей с отложенным оповещением: изменения происходят внутри замыканий обновления сущности, а фреймворк оповещает зависимых после возврата из замыкания. Синхронные подписки `Field<T>` с этой моделью напрямую не складываются, поэтому интеграция соединяет их через асинхронный канал.

## Как это работает

`amethystate-gpui` даёт `RpView<T>` — обёртку, которая держит срез состояния и `ReactiveScope`. При создании она подписывается на все внешние изменения этого среза и отправляет пустое сообщение в неограниченный канал. Фоновая задача внутри сущности вычерпывает этот канал и вызывает `entity_cx.notify()`, что запускает перерисовку GPUI.

То есть GPUI читает состояние синхронно во время `render` через `.get()`, а изменения обнаруживаются асинхронно в фоне.

## Установка

```toml
[dependencies]
amethystate-gpui = "*"
```

Инициализируйте хранилище до того, как открывать окна:

```rust
StoreBuilder::new("./app.redb").init_global();
```

## Объявление состояния

```rust
#[amethystate(prefix = "counter")]
pub struct CounterState {
    #[amestate(default = 0)]
    pub count: i32,
}
```

## Создание сущности

Чтобы обернуть срез состояния в `RpEntity`, используйте `cx.new_amethystate()` вместо `cx.new()`:

```rust
struct CounterView {
    state: RpEntity<CounterState>,
}

impl CounterView {
    fn new(cx: &mut Context<Self>) -> Self {
        let state = cx.new_amethystate(CounterState::new).unwrap();
        Self { state }
    }
}
```

`RpEntity<T>` - псевдоним для `Entity<RpView<T, Store>>`. `RpView` разыменовывается в `T`, поэтому к полям состояния обращаются прямо через сущность.

## Чтение состояния в render

```rust
impl Render for CounterView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_count = self.state.read(cx).count().get();

        div().child(format!("Count: {}", current_count))
    }
}
```

## Запись состояния

Писать можно откуда угодно — из обработчика `on_click` сущности, из фонового потока, из другой части приложения. Любая внешняя запись запускает `notify()` и перерисовку:

```rust
// from a click handler inside render
let state = self.state.clone();
Button::new("Increment")
    .on_click(move |_, _, cx| {
        state.read(cx).count().update(|v| v + 1).ok();
    })

// from a background thread via fork
let forked = state.read(cx).fork();
std::thread::spawn(move || {
    loop {
        std::thread::sleep(Duration::from_secs(2));
        forked.count().update(|v| v + 1).ok();
    }
});
```

Учтите, что записи с того же экземпляра (без форка) не запускают подписку `external` и потому сущность не оповещают. Пишете из фонового потока и хотите, чтобы интерфейс отреагировал, - берите `.fork()`.

## Использование со своей версией GPUI

Если ваш проект зависит от git-версии GPUI, добавьте `[patch]` в `Cargo.toml` рабочего пространства, чтобы использовалась одна копия крейта:

```toml
[patch.crates-io]
gpui = { git = "https://github.com/zed-industries/zed", rev = "abc123" }
```

Без этого Cargo сочтёт версию с crates.io и версию из git разными крейтами, и при компиляции вы получите ошибки несовпадения типов.

## Примеры

- [`gpui-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/gpui-settings)
