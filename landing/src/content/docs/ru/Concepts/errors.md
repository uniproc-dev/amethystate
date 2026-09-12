---
title: Что несёт в себе ошибка
sidebar:
  label: Ошибки
  order: 20
---

**Каждая точка апи падает тем набором, который в ней возможен.** Конструктор
отвечает пятью способами, которыми структура отказывается открыться; запись
через `Kv` — шестью, которыми заворачивают сырую запись. Набор их перечисляет —
значит, `match` по нему полный, и компилятор такой его и держит.

Каждый набор — обычный `std::error::Error`. Поэтому с отказом делают одно из
двух, и оба дёшевы: разбирают его или отдают дальше.

## Разобрать отказ

<!-- shown: telling one refusal from another -->
```rust
let refused = match Panel::new_with(&store) {
    Ok(_) => return Ok(()),
    Err(why) => why,
};

let said = match refused {
    OpenStruct::Refused { at, said } => format!("{at} was turned down: {said}"),
    OpenStruct::WillNotRead { at, why } => format!("{at} holds something else: {why}"),
    OpenStruct::Taken(taken) => format!("{} already holds it", taken.held_by),
    OpenStruct::NotAPath(why) => format!("that is not a path: {why}"),
    OpenStruct::Store(disk) => format!("the store: {disk}"),
};
```
<!-- /shown -->

Ветки `_` нет, и ради этого всё и сказано типом. Заглушка `_` — это место, куда
незаметно проваливается тот, кто ждал «отказали», а встретил чуть другое; здесь
ей нечего ловить. А когда у вызова появится новый способ упасть, у всех, кто
разбирал все, перестанет компилироваться — а именно это им и надо сообщить.

Все эти наборы исчерпывающие, по той же причине.

**Вариант несёт всё, что было известно там, где его подняли:** место вместе с
владельцем, место вместе с причиной, обе стороны столкновения.
`OpenStruct::Taken` несёт четыре вещи — какое место, кто его хотел, через
какой путь оно занято и кем, — потому что столкновение читается только по всем
четырём, а друг о друге объявления не знают. Лишнее поле можно не смотреть;
нужное на месте.

## Отдать его дальше

<!-- shown: letting the caller's own error type take it -->
```rust
fn with_anyhow(store: &amethystate::Store) -> anyhow::Result<()> {
    store.set(["ui", "width"], &800u32)?;
    Ok(())
}

fn with_a_box(store: &amethystate::Store) -> Result<(), Box<dyn Error + Send + Sync>> {
    store.set(["ui", "height"], &600u32)?;
    Ok(())
}
```
<!-- /shown -->

`Send + Sync + 'static`, `Display` в одну строку и `source()`, уходящий к
причине. Поэтому `anyhow`, `eyre` и `Box<dyn Error>` берут набор простым `?`, и
махнуть рукой ничего не стоит.

## Наборы

| набор | откуда |
| --- | --- |
| `OpenStruct` | `new`, `new_with`, `new_with_id`, `new_with_id_under`, `load`, `load_with`, `Kv::cell` |
| `OpenStore` | `StoreBuilder::build`, `build_with_migration`, `located` |
| `LoadMap` | `Kv::map` и собственный конструктор поля-карты |
| `ReadValue` | `Store::get` |
| `WriteValue` | `Store::set`, `Store::delete`, `Field::set`, `ReactiveCell::set`/`update`/`modify`, `ReactiveMap::insert` |
| `KvWrite` | `Kv::get`, `Kv::set`, `Kv::remove` |
| `ScanKeys` | `Store::scan_keys`, `Store::scan_prefix`, `Kv::keys` |
| `Flush` | `save_now`, `close`, `flush_prefix` |
| `RunStep` | каждый метод `MigrationContext` и то, что отдаёт сам шаг |

Они пересекаются и всё равно остаются разными типами. У `WriteValue` есть
`Intercepted`, `Absent` и `SourceGone`, до которых сырая запись через `Kv` не
дотягивается; у `KvWrite` есть `Declared`, до которого не дотягивается запись
через поле. Общих вариантов четыре. Одним набором каждый вызывающий читал бы
ветки, которые у него выстрелить не могут.

У карты набор свой, а не общий с `OpenStruct`, потому что её открывают поверх
того, что уже лежит под ней, — и она встречает два отказа, которых больше не
встречает никто: сохранённый ключ, который ей не запись, и запись, чьё *имя* не
читается как тип ключа. У шага миграции набор свой, потому что различию, которое
ему нужно, — «эта запись не то, что я ждал», мимо чего шаг часто может пройти,
против «диск сломан», мимо чего не может, — больше негде жить.

`Field::try_get` стоит особняком, и намеренно: это не запись и не отказ
хранилища, а то, о чём поле и хранилище не договорились. Он отвечает
`Disagreement` — путь и одна из четырёх причин, — и это тоже обычный `Error`.
Что значит каждая причина:
[Объявление структур](/amethystate/ru/state/defining-structs/#значение-декодируется-и-оно-бессмысленно).

## Отчёт никуда не делся

Вариант `Store(..)` любого набора несёт то, что сказал движок, целиком — как
`Report` из [`error_stack`](https://docs.rs/error-stack). И каждый вариант,
который отчёт классифицировал, тоже: `WillNotRead`, `WillNotEncode` и `TooDeep`
держат отчёт, а не отрендеренное предложение, потому что цифры и есть диагноз.

<!-- shown: the report under a variant that named the failure -->
```rust
let refused = store.get::<u16>(["port"]).unwrap_err();

let amethystate::store::ReadValue::WillNotRead { at, why } = refused else {
    panic!("the bytes are there and they are not a u16")
};
```
<!-- /shown -->

Частей у отчёта две, и enum из них только первая:

- **цепочка контекстов** — что упало, на каждом уровне, который знал, что
  падает;
- **вложения** — подробности, которые несут типами, а не предложениями.

Достать отчёт из набора — такой же `match`, как любой другой:

<!-- shown: getting at the report a set carries -->
```rust
fn what_the_store_said(why: LoadMap) -> StorageResult<()> {
    match why {
        LoadMap::EntryWillNotRead { why, .. } => Err(why.into_report()),
        other => panic!("an entry was expected to be at fault: {other}"),
    }
}
```
<!-- /shown -->

### Вершина называет операцию

<!-- shown: what a failure says it is -->
```rust
let refused = store.kv().map::<String, u64>("labels").unwrap_err();
let report = what_the_store_said(refused).unwrap_err();

let context = report.current_context();
let sentence = report.to_string();
```
<!-- /shown -->

`context` — это `StorageError::Codec`, а `sentence` — *the value could not be
encoded or decoded*.

`current_context()` — самый внешний контекст. Варианты `StorageError` называют
**операцию, которая упала**, а не то, обо что она упала: `Write`, `Scan`,
`Codec`. Два разных движка, не сумевших записать, дают один и тот же контекст, а
различаются кадрами под ним, — и это сделано нарочно: тому, кто решает, что
делать дальше, важно, легла ли запись, а не redb или `serde_json` сказал «нет».

`StorageError` **помечен** `#[non_exhaustive]`, в отличие от наборов. Это
собственный список диска, он растёт вместе с движками, и его читают, а не
разбирают по веткам.

### Подробности приложены типами

Каждый факт — свой тип, поэтому из сообщения ничего не парсится обратно. Живут
они в `amethystate::errors::facts`: ключ, префикс, под которым шло сканирование,
запись, на которой оно споткнулось, файл, который читали, размер значения.
Каждый — newtype над тем, что несёт, а подпись живёт в его `Display`.

`facts::all::<T, _>` отдаёт все факты одного типа, начиная с самого глубокого:

<!-- shown: reaching the entry that failed -->
```rust
let refused = store.kv().map::<String, u64>("ports").unwrap_err();
let report = what_the_store_said(refused).unwrap_err();

let entries: Vec<&Entry> = facts::all::<Entry, _>(&report).collect();
let prefixes: Vec<&Prefix> = facts::all::<Prefix, _>(&report).collect();
```
<!-- /shown -->

В `entries` один `Entry("http")`, в `prefixes` один `Prefix("ports")`. Карта, не
открывшаяся из-за одной плохой записи, говорит, из-за какой именно, — и это
ровно то, что печать через `{}` выбрасывает.

Спросить факт, которого в отчёте нет, — получить ничего:

<!-- shown: asking for a fact the report does not carry -->
```rust
let refused = store.kv().map::<String, u64>("ports").unwrap_err();
let report = what_the_store_said(refused).unwrap_err();

let key = facts::all::<Key, _>(&report).next();
```
<!-- /shown -->

`key` — это `None`: этот отчёт никогда не был про один ключ.

Какие факты в отчёте, зависит от того, кто был на стеке: их прикладывает тот,
кто их знал, и `Key` не приложит код, видевший только префикс. Читать их надо
как улики, которые есть, когда есть, — не как схему. Прикладывают лениво: на
успешном пути не строится ничего.

Это тот слой, от которого наборы и избавляют. Лезть сюда стоит, когда пишешь
строчку в лог или отчёт о баге, — а не когда решаешь, что делать дальше.

## Как это печатается

`{}` набора — одна строка, написанная для того, кому это чинить. `{:?}` отчёта
под ним даёт всё: каждый контекст цепочки и факты под тем кадром, который их
приложил.

<!-- shown: an entry that will not decode -->
```rust
store.set(["ports", "http"], &"text".to_string())?;

let undecodable = store.kv().map::<String, u64>("ports").unwrap_err();
```
<!-- /shown -->

<!-- printed: an entry that will not decode from book_errors -->
```
the entry at ports.http will not read back

the value could not be encoded or decoded
├╴as: u64
├╴value bytes: 5
├╴prefix: ports
├╴entry: http
│
├─▶ Erased codec error: wrong msgpack marker FixStr(4)
│
╰─▶ wrong msgpack marker FixStr(4)
```
<!-- /printed -->

<!-- shown: an entry whose name is not the map's key type -->
```rust
let wrong_key = store.kv().map::<Id<u16>, String>("ports").unwrap_err();
```
<!-- /shown -->

<!-- printed: an entry whose name is not the map's key type from book_errors -->
```
`http` under ports will not read as a Id<u16>
```
<!-- /printed -->

Под этим отчёта нет, и не надо: вариант уже назвал карту, запись и тип, которым
её имя не прочиталось.

<!-- shown: a path that names nothing -->
```rust
let names_nothing = store.set(Vec::<String>::new(), &1u32).unwrap_err();
```
<!-- /shown -->

<!-- printed: a path that names nothing from book_errors -->
```
the write was given no path to land at: a path of no levels names the whole store - say `StorePath::root()` to mean it
```
<!-- /printed -->

<!-- shown: a path past the cap it was given -->
```rust
let shallow = StoreBuilder::new(settings)
    .limits(|l| l.key_depth(4))
    .build()?;

let too_deep = shallow.set(["a", "b", "c", "d", "e"], &1u32).unwrap_err();
```
<!-- /shown -->

<!-- printed: a path past the cap it was given from book_errors -->
```
a.b.c.d.e is deeper than this store reads back

deeper than this store reads back
├╴key: a.b.c.d.e
├╴levels: 5, and the limit is 4
├╴set by: limits(|l| l.key_depth(..))
╰╴what is stored here spends the same budget - this store reads 512 levels in all
```
<!-- /printed -->

Последний — образец, к которому стоит стремиться, когда отказ пишешь ты сам:
предложение называет, что отвергли, а факты под ним отвечают на вопрос, который
читатель сейчас задаст, — чей это был предел и во что он обошёлся.

Значит, `{:?}` отчёта — это то, что кладут в лог, а `{}` набора — та самая одна
строка, которую читает человек.

## Когда нужен именно `std::error::Error`

`Report` этот трейт не реализует, поэтому под баунд `E: Error` он не подходит —
а его требует и `anyhow::Error::new`, и `thiserror` через `#[source]`, и любая
своя обёртка над чужой ошибкой. Наборы этот баунд удовлетворяют, поэтому у
границы вопрос не встаёт; встаёт он, когда отчёт уже вынут и переходить надо
ему самому. Дорога — `into_error`, и она ничего не теряет:

<!-- shown: turning a report into a std error -->
```rust
let std_error = report.into_error();

let sentence = std_error.to_string();
let whole = format!("{std_error:?}");
```
<!-- /shown -->

`sentence` — то же самое *the value could not be encoded or decoded*, а в
`whole` по-прежнему лежит `entry: http`. Обёртка держит отчёт за собой, а не
расплющивает его. `as_error` — заимствующий близнец, чтобы отдать отчёт, не
отдавая его насовсем.

`error_stack` переэкспортирован как `amethystate::error_stack`, а `Report` лежит
ещё и в `amethystate::errors`, — так что назвать его в своей сигнатуре не стоит
ни одной своей зависимости и не разъедется с этим крейтом по версиям.
