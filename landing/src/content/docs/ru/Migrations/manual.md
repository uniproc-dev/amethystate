---
title: Ручные миграции
sidebar:
  order: 22
---

Кодогенерируемые миграции покрывают обычный случай: переименовать поля, поменять типы, заполнить значения по умолчанию. Когда этого мало — чтения из другого узла, дозаполнение данных, чистка ключей внутри `ReactiveMap`, — шаги миграции можно написать руками.

## Точка входа

Ручные шаги регистрируются через метод билдера `.migrations()`:

```rust
let (store, report) = StoreBuilder::new("./app.redb")
    .migrations(|m| {
        m.collect_codegen(); // include all #[migrate] steps
        // register manual steps here
    })
    .build()?;
```

`collect_codegen()` затягивает все шаги, объявленные атрибутом `#[migrate]`. Вызывать его вместе с ручными шагами можно в любом порядке — порядок выполнения мигратор всё равно определяет топологической сортировкой.

## Объявление шага

```rust
m.for_node::<Profile>()
    .step(2, "split full name", |ctx| {
        let full_name = ctx.get::<String>("full_name")?.unwrap_or_default();
        let mut parts = full_name.splitn(2, ' ');
        ctx.set("first_name", &parts.next().unwrap_or_default().to_string())?;
        ctx.set("last_name", &parts.next().unwrap_or_default().to_string())?;
        ctx.delete("full_name")?;
        Ok(())
    });
```

`for_node::<T>()` нацеливается на структуру по её префиксу. `.step(version, description, closure)` регистрирует преобразование, которое переводит её из `version - 1` в `version`.

## API контекста

Внутри замыкания шага `ctx` даёт низкоуровневый доступ к сохранённым ключам узла. Все аргументы-ключи отсчитываются от префикса узла, если не сказано иначе.

### Базовые операции

| Метод | Описание |
|--------|-------------|
| `ctx.get::<T>(key)` | Читает ключ. Возвращает `Result<Option<T>>`. |
| `ctx.set(key, value)` | Пишет ключ. |
| `ctx.delete(key)` | Удаляет ключ. |
| `ctx.rename(from, to)` | Копирует ключ под новое имя и удаляет старое. Ничего не делает, если исходного ключа нет. |

### Составные операции

Они объединяют чтение, преобразование и запись в один вызов:

| Метод | Описание |
|--------|-------------|
| `ctx.transform::<TOld, TNew>(key, f)` | Читает ключ, применяет `f`, пишет результат обратно под тем же ключом. Ничего не делает, если ключа нет. |
| `ctx.merge::<T1, T2, TNew>((k1, k2), into, f)` | Читает два ключа, соединяет их через `f` в третий ключ, удаляет исходные. Ничего не делает, если нет хотя бы одного из источников. |
| `ctx.split::<TOld, T1, T2>(from, (k1, k2), f)` | Читает один ключ, разделяет его на два через `f`, удаляет исходный. Ничего не делает, если источника нет. |

Примеры:

```rust
// multiply a value in place
ctx.transform::<u16, u16>("sample_rate", |r| Ok(r.saturating_mul(10)))?;

// combine first_name + last_name into display_name, remove originals
ctx.merge::<String, String, String>(
    ("first_name", "last_name"),
    "display_name",
    |f, l| Ok(format!("{f} {l}")),
)?;

// split "host:port" into two separate keys
ctx.split::<String, String, u16>(
    "address",
    ("host", "port"),
    |s| {
        let (h, p) = s.split_once(':').unwrap();
        Ok((h.to_string(), p.parse()?))
    },
)?;
```

### Коллекции

| Метод | Описание |
|--------|-------------|
| `ctx.scan_map::<K, V>(key)` | Обходит все записи под `prefix.key.*` и возвращает их как `IndexMap<K, V>` — в том порядке, в каком по ним идёт сама карта. |

Полезно, когда поле `ReactiveMap` мигрируют, не проходя через `AmeData`:

```rust
let old_routes = ctx.scan_map::<String, String>("routes")?;
for (k, _) in &old_routes {
    ctx.delete(&format!("routes.{k}"))?;
}
```

### Глобальный доступ

| Метод | Описание |
|--------|-------------|
| `ctx.global_get::<T>(path)` | Читает любой ключ из хранилища по его полному пути. |
| `ctx.global_set(path, value)` | Пишет любой ключ в хранилище по его полному пути. |

`global_get` и `global_set` полностью обходят префикс узла. Полезно, когда шагу нужно прочитать из узла, который уже мигрировал:

```rust
let plan = ctx.global_get::<String>("identity.plan")?.unwrap();
```

### Ограничение области

`ctx.scoped(sub_prefix)` возвращает новый `MigrationContext` с корнем в `{current_prefix}.{sub_prefix}`. Используется внутри `ctx.nested()` и напрямую нужен редко.

## Зависимости между узлами

Если шаг читает из другого узла через `ctx.global_get`, тот узел уже должен был мигрировать. Объявите зависимость явно через `.depends_on()`:

```rust
m.for_node::<Profile>()
    .depends_on::<Identity>()
    .step(2, "snapshot plan from identity", |ctx| {
        let plan = ctx
            .global_get::<String>("complex_identity.plan")?
            .expect("identity should have migrated first");
        ctx.set("plan_snapshot", &plan)?;
        Ok(())
    });
```

Мигратор использует объявленные зависимости, чтобы определить порядок выполнения. Если у самого `Identity` есть шаги, они гарантированно завершатся до того, как выполнится этот шаг.

Зависимости складываются в граф. Цепочка вида `Workspace` → `Profile` → `Identity` означает, что все три мигрируют по порядку, независимо от порядка регистрации.

## Доступ к MigrationContext из #[migrate]

Когда кодогенерируемой миграции нужно почистить ключи, которые `AmeData` не покрывает, — например, удалить старые записи из `ReactiveMap`, — функция `#[migrate]` может принять `MigrationContext` вторым аргументом:

```rust
#[migrate]
fn migrate_proxy_config_v1_to_v2(
    old: AmeData<v1::ProxyConfig>,
    ctx: &mut MigrationContext,
) -> amethystate::MigrationResult<AmeData<ProxyConfig>> {
    for key in old.routes.keys() {
        ctx.delete(&format!("routes.{}", key))?;
    }

    let endpoints = old.routes
        .into_iter()
        .filter(|(k, _)| k != "obsolete")
        .map(|(k, v)| (k, ProxyEndpoint { url: v, timeout_ms: 5000 }))
        .collect();

    Ok(AmeData::<ProxyConfig> {
        name: old.name,
        endpoints,
    })
}
```

Без явных вызовов `ctx.delete` старые ключи `routes.*` остались бы в хранилище после миграции. `AmeData` покрывает только те поля, которые есть в структуре; всё остальное приходится чистить руками.

## Падение и откат

Узлы, связанные зависимостями, группируются в одну транзакцию. Если падает любой шаг в группе, все изменения группы откатываются. Узлов в других группах это не касается.

```
❌ Component ["complex_broken_child", "complex_broken_root"] failed: Migration error: intentional failure
   Transaction rolled back. Data for these prefixes remains unchanged.
```

После упавшего компонента хранилищем всё ещё можно пользоваться. Узлы, которые мигрировали успешно, доступны. Узлы в упавшем компоненте остаются на прежней версии.

Полный итог доступен в `MigrationReport`, который возвращает `.build()`:

```rust
let (store, report) = StoreBuilder::new("./app.redb")
    .migrations(|m| { ... })
    .build()?;

if report.has_failures() {
    for component in &report.components {
        // component.outcome, component.prefixes
    }
}
```
