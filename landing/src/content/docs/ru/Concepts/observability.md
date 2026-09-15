---
title: Наблюдаемость
sidebar:
  order: 19
---

`amethystate` шлёт структурированные события трассировки через крейт [`tracing`](https://docs.rs/tracing). У всех событий таргет `amethystate`, так что фильтровать их можно отдельно от остального приложения.

## Как включить трассировку

События идут на уровне `TRACE`. Чтобы увидеть их, настройте подписчика `tracing` с включённым таргетом `amethystate`:

```rust
tracing_subscriber::fmt()
    .with_env_filter("amethystate=trace")
    .init();
```

Или через переменную окружения `RUST_LOG`:

```sh
RUST_LOG=amethystate=trace cargo run
```

## Что попадает в лог

### Записи в поля

Каждый `field.set()` даёт одно событие:

```
TRACE amethystate: field write path=network.port source=NetworkState
TRACE amethystate: field write path=network.port source=external
```

| Поле | Значение |
|-------|-------|
| `path` | Ключ поля в store, например `network.port` |
| `source` | Короткое имя структуры, которая позвала `set()`, или `external`, если запись пришла мимо процесса: наблюдатель за файлом, другой процесс, миграция |

### Срабатывания подписок

Каждый вызов колбэка подписки даёт одно событие:

```
TRACE amethystate: signal emit → subscription fire subscription_id=0 name=PortWatcher location=src/main.rs:42
TRACE amethystate: map signal emit → any subscription fire subscription_id=1 name=None location=src/server.rs:88
```

| Поле | Значение |
|-------|-------|
| `subscription_id` | Монотонное целое: опознаёт подписку внутри её сигнала |
| `name` | Метка из `.named()`, или `None` |
| `location` | `file:line` того места, где позвали `.subscribe()` |

## Как помечать подписки

Место вызова подписка запоминает сама, через `#[track_caller]`. Метку человеческими словами добавляют через `.named()` — для подписок из обобщённого кода или из кода фреймворка, где файл и строка сами по себе ничего не скажут:

```rust
let _sub = state.port()
    .subscribe(|p| do_something(p))
    .named("PortWatcher");

// ReactiveMap works the same way
let _sub = state.limits()
    .subscribe_any(|change| handle(change))
    .named("LimitsWatcher");
```

Дальше она стоит в каждом событии этой подписки.

## Как читать вывод

Запись, за которой сработали два подписчика, выглядит так:

```
TRACE amethystate: field write path=network.port source=NetworkState
TRACE amethystate: signal emit → subscription fire subscription_id=0 name=PortWatcher location=src/ui.rs:55
TRACE amethystate: signal emit → subscription fire subscription_id=1 name=None location=src/logger.rs:12
```

`source=external` значит, что изменение пришло снаружи: наблюдатель заметил, что файл store правили, или шаг миграции записал значение на старте.
