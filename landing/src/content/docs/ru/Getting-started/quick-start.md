---
title: Быстрый старт
sidebar:
  order: 3
---

Кратчайший путь от пустого места до работающего store. На каждом шаге —
ссылка на раздел, где этот шаг разобран как следует.

Всё, что импортируется ниже, лежит в прелюдии, и обычная программа берёт её
целиком:

```rust
use amethystate::prelude::*;
```

В ней есть всё, что нужно, чтобы объявить, открыть, прочитать и записать, — в
том числе `StoreExt` и `StoreBackend`. Это трейты, и без них у store как будто
нет ни `get`, ни `save_now`. В примерах ниже импорты выписаны поимённо, чтобы
было видно, откуда что берётся.

## Объявите состояние

Один атрибут — и поля структуры живут в store и будят подписчиков. `prefix`
задаёт, где они лежат.

<!-- shown: declaring a state struct -->
```rust
use amethystate::amethystate;

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
```
<!-- /shown -->

Значения по умолчанию, вложенные структуры, volatile-поля, политики
чтения и serde:
[Объявление структур](/amethystate/ru/state/defining-structs/).

## Создайте store

<!-- shown: opening a store you hold yourself -->
```rust
let store = StoreBuilder::new(settings)
    .disk(|d| d.debounce(Duration::from_millis(500)))
    .build()?;

let state = NetworkState::new_with(&store)?;
```
<!-- /shown -->

`new_with` принимает параметром `Store` — тот, что вы держите сами. Как открыть
один на весь процесс, дать платформе выбрать место для файла и что вернёт
`close`, если последний сброс не удался:
[Открытие store](/amethystate/ru/store/opening/).

Задержки, повторные попытки и то, чего store не примет:
[Настройка store](/amethystate/ru/store/configuration/). В какой файл пишет
движок: [Установка](/amethystate/ru/getting-started/installation/).

## Читайте, пишите, подписывайтесь

<!-- shown: reading, writing and subscribing -->
```rust
println!("{}", state.host().get());

let _sub = state.port().subscribe(|port| {
    println!("port changed to {port}");
});

state.port().set(9090)?;
```
<!-- /shown -->

Запись доходит до памяти сразу, до диска — с задержкой. Как звать колбэки в
своём потоке, отсеивать собственные записи и сколько стоит подписка:
[Подписки](/amethystate/ru/concepts/subscriptions/).

Кому нужна запись сейчас, а не «когда-нибудь потом»:
[Durability](/amethystate/ru/concepts/durability/).

## Ключи, которые появляются в рантайме

Карта кладёт каждую запись по своему пути — добавлять и слушать можно по
одной.

<!-- shown: a map whose keys are not known up front -->
```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertThresholds {
    pub warning: u64,
    pub critical: u64,
}

#[amethystate(prefix = "sys")]
pub struct SystemSettings {
    #[amestate(default = {
        "cpu": AlertThresholds { warning: 70, critical: 90 },
        "mem": AlertThresholds { warning: 80, critical: 95 }
    })]
    pub limits: ReactiveMap<String, AlertThresholds>,
}
```
<!-- /shown -->

<!-- shown: working with a map -->
```rust
state.limits().insert(
    "gpu".to_string(),
    &AlertThresholds {
        warning: 60,
        critical: 85,
    },
)?;

let cpu = state.limits().get("cpu");

for (key, value) in state.limits().entries() {
    println!("{key}: {value:?}");
}

let _sub = state.limits().subscribe_any(|change| {
    println!("{change:?}");
});
```
<!-- /shown -->

`entries()` отдаёт записи в порядке store — то есть по именам, которые ключи
одалживают. Нестроковый ключ идёт через `Id` и упорядочен по написанию, которое
`Id` держит, поэтому `Id<u16>` приходит как `10, 100, 9`.

Пути, которые складываются уже в рантайме, обходятся без структуры: их держит
[Kv](/amethystate/ru/primitives/kv/). А когда такой путь один и за его значением
надо следить, [ReactiveCell](/amethystate/ru/primitives/reactive-cell/) даёт по
нему то же, что поле даёт по объявленному, — чтение, запись и подписку.

## Когда структура меняется

Поднимите `version` и объявите шаги между версиями — тогда библиотека перенесёт
данные, записанные старой сборкой. Поле переименовали или сменили ему тип, а
версию не подняли — это дрейф: о нём скажут, и запуск продолжится.

Открывайте store через `build_with_migration` всегда, когда в бинарнике
есть `#[migrate]`: `build` прогоняет только те шаги, которые вы передали ему
явно.

[Миграции](/amethystate/ru/migrations/overview/).

## Режим Persistent-only

Для фреймворков с явным циклом отрисовки/обновления — egui, iced, ratatui —
`mode = "persistent"` оставляет поля обычными: сохраняются вручную.

<!-- shown: a struct in persistent mode -->
```rust
#[amethystate(prefix = "kept", mode = "persistent")]
pub struct KeptSettings {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
```
<!-- /shown -->

<!-- shown: writing a persistent struct -->
```rust
let mut state = KeptSettings::load_with(&store)?;

state.port = 9090;
state.save()?;

state.mutate(|d| {
    d.host = "0.0.0.0".to_string();
    d.port = 443;
})?;
```
<!-- /shown -->

`save_lazy` и `mutate_lazy` делают то же самое, но на диск сбрасывает
дебаунсер.

## Что ещё есть

- **Перехватчики** — колбэки, которые видят запись до того, как она ляжет, и
  могут переписать её или отказать:
  [Подписки](/amethystate/ru/concepts/subscriptions/).
- **Трассировка** — структурированные события: каждая запись помечена
  структурой, которая её сделала:
  [Наблюдаемость](/amethystate/ru/concepts/observability/).
- **Интеграции с фреймворками** — Tauri с биндингами TypeScript, Leptos,
  Dioxus, Yew, GPUI, windows-reactor:
  [Интеграции](/amethystate/ru/integrations/overview/).
