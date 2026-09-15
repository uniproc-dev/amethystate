---
title: Typescript
---

`amethystate` даёт пакет для TypeScript, для приложений Tauri с фронтендом на чистом TypeScript или JavaScript. Пакет поставляет `ReactiveField<T>`, `ReadonlyReactiveField<T>` и `ReactiveMap<K, V>` — те примитивные классы, поверх которых построены сгенерированные биндинги, — плюс объединение `MapChange<K, V>`, которое доставляют подписки на карты.

## Установка

```sh
npm install amethystate
```

`@tauri-apps/api` - peer-зависимость, и она уже должна быть в вашем проекте.

## Кодогенерация

Сгенерированные биндинги - это один файл TypeScript, который импортирует из `amethystate` и открывает типизированные классы для каждого вашего среза состояния.

**1. Добавьте бинарную цель и зависимость в крейт Tauri:**

```toml
# src-tauri/Cargo.toml
[[bin]]
name = "codegen"
path = "src/bin/codegen.rs"

[dependencies]
amethystate-codegen = { version = "0.20" }
```

**2. Создайте `src/bin/codegen.rs`:**

```rust
#[allow(unused_imports)]
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    ts_out = "../src/bindings/amethystate.ts"
);
```

**3. Запустите:**

```sh
cargo run --bin codegen
```

## Использование сгенерированных биндингов

Каждая корневая структура становится классом со статическим методом `load()`. Вызовите его один раз при запуске, до отрисовки интерфейса.

```ts
import { AppSettings } from "./bindings/amethystate";

const settings = await AppSettings.load();
```

`load()` читает пачкой все ключи под префиксом среза одним вызовом IPC и заводит подписки, чтобы локальный кэш оставался в согласии с бэкендом.

Вложенная структура становится собственным классом, держащим поля этой структуры, и до него добираются через обращение к свойству:

```ts
settings.theme.mode.value = "dark";
```

## Чтение и запись полей

Обычное поле - это экземпляр `ReactiveField<T>` с двумя способами доступа:

```ts
// synchronous — reads from the local in-memory cache
const name = settings.username.value;

// optimistic write — updates cache immediately, persists asynchronously
settings.username.value = "Alice";

// async — reads directly from the persistent store (transaction-safe)
const storedName = await settings.username.get();

// async write — queues a write to the store
await settings.username.set("Alice");
```

Геттер и сеттер `value` - обычный выбор для привязок интерфейса. Асинхронные методы берите тогда, когда нужна гарантия, что значение согласовано с бэкендом, или когда нужен явный контроль над тем, когда запись поставлена в очередь.

Геттер `value` типизирован как `T | null`. Он читается как `null`, пока не придёт первое значение, а для ключа, который есть в хранилище, это уже сделано к моменту, когда `load()` завершится.

`ReadonlyReactiveField<T>` - тот же класс без сеттера `value` и без `set()`.

## Подписки

```ts
const unsubscribe = settings.username.subscribe((val) => {
    console.log("username changed:", val);
});

// later
unsubscribe();
```

Возвращённая функция сама может вернуть промис. Дождитесь его, когда нужно, чтобы отписка на бэкенде завершилась до того, как вы продолжите.

## Сброс на диск

Записи буферизуются в фоне с дебаунсом. Чтобы гарантировать немедленное сохранение — например, перед закрытием приложения, — вызовите `save()` на срезе:

```ts
await settings.save();
```

## ReactiveMap

Поле-карта - это экземпляр `ReactiveMap<K, V>`. `K` ограничен `string`. Он даёт синхронный и асинхронный доступ, удаление и подписки как по одному ключу, так и на всю карту:

```ts
// async
await settings.env.set("HTTP_PROXY", "http://localhost:8080");
const proxy = await settings.env.get("HTTP_PROXY");

// synchronous (in-memory cache)
settings.env.setSync("HTTP_PROXY", "http://localhost:8080");
const cachedProxy = settings.env.getSync("HTTP_PROXY");
const hasProxy = settings.env.hasSync("HTTP_PROXY");

// iterate current entries
for (const [key, val] of settings.env.entries) {
    console.log(key, val);
}

// async delete — resolves once the backend confirms
await settings.env.remove("HTTP_PROXY");

// optimistic delete — drops the cache entry, deletes in the background
settings.env.removeSync("HTTP_PROXY");

// subscribe to any change
const unsubAny = settings.env.subscribeAny((change) => {
    if (change.type === "Insert") { /* change.key, change.value */ }
    if (change.type === "Update") { /* change.key, change.oldValue, change.newValue */ }
    if (change.type === "Remove") { /* change.key, change.oldValue */ }
    if (change.type === "Clear")  { /* no payload */ }
});

// subscribe to a specific key
const unsubKey = settings.env.subscribeKey("HTTP_PROXY", (val) => {
    console.log("proxy changed:", val);
});
```

И `get()`, и `getSync()` возвращают `V | null` для ключа, которого карта не держит. `entries` - это `ReadonlyMap<K, V>` поверх локального кэша.

## Очистка

Каждое поле и каждая карта регистрируют подписку, когда их создают. Вызовите `destroy()` на каждом, чтобы снять регистрацию:

```ts
settings.username.destroy();
settings.theme.mode.destroy();
settings.env.destroy();
```

Сгенерированный класс среза открывает `load()` и `save()`, поэтому очистка идёт по полям, а не по срезу.

## Примеры

- [`tauri-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-settings) — фронтенд на TypeScript
