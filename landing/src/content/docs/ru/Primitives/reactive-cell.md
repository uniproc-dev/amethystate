---
title: "ReactiveCell<T>"
sidebar:
  label: ReactiveCell
  order: 10
---

Тип, в который стирается любое реактивное значение: поле, запись карты, путь
без структуры за ним, обычное значение в памяти. Коду, которому нужен «`u64`,
который я умею читать, писать и слушать», знать, что из четырёх за ячейкой
стоит, незачем — как и тащить движок store и режим доступа в свою сигнатуру.

<!-- shown: four ways to reach a cell -->
```rust
let width = state.sidebar_width().cell();
let cpu_column = state.widths().entry_cell("cpu".to_string());
let by_path = store.kv().cell("dragging", 0u64)?;
let loose = ReactiveCell::new(0u64);

let mut columns: HashMap<String, ReactiveCell<u64>> = HashMap::new();
columns.insert("sidebar".to_string(), width);
columns.insert("cpu".to_string(), cpu_column);
columns.insert("dragging".to_string(), by_path);
columns.insert("loose".to_string(), loose);
```
<!-- /shown -->

Три из них пишут насквозь в store. `ReactiveCell::new` держит значение в
памяти, и процесс его не переживает. Ячейка от поля с `#[amestate(volatile)]`
устроена так же: у поля нет подписки на store, через которую ячейка могла бы
зафиксировать запись.

`kv.cell` обходится без объявления вовсе: путь и значение по умолчанию он берёт
прямо в вызове, а тип запоминает до конца прогона — второй вызов по тому же
пути с другим типом получит отказ. [Kv](/amethystate/ru/primitives/kv/).

## Чтение

```rust
fn get(&self) -> Option<T>
```

`get` отвечает `None` там, где значение взять неоткуда: запись карты, чьего
ключа нет, или ячейка, чей источник пропал. У
[`Field`](/amethystate/ru/primitives/field/) на этом месте `default`, поэтому
значение у него есть всегда.

Читает `get` кэш, который держит сама ячейка, — стоит это столько же, сколько
прочитать примитив напрямую, и дёшево настолько, что годится для цикла,
который читает каждый кадр.

<!-- shown: reading, writing and watching a cell -->
```rust
let current = cell.get();

let _sub = cell.subscribe(|width| println!("width -> {width:?}"));

cell.set(200)?;
cell.update(|width| width + 10)?;
cell.modify(|width| *width += 10)?;
```
<!-- /shown -->

## Записи ложатся туда, где живёт значение

Ячейка пишет насквозь в то, что за ней стоит. Ячейки, чья запись доходит до
кэша и там застревает, взять негде.

По дороге туда кэш не трогают: он обновится, когда store скажет, что
зафиксировал. Поэтому отклонённая запись в `get` не всплывёт.

## Ячейка на запись карты

<!-- shown: a cell onto a map entry -->
```rust
state.widths().insert("cpu".to_string(), &120)?;

let cpu = state.widths().entry_cell("cpu".to_string());
let absent = state.widths().entry_cell("gpu".to_string());

assert_eq!(cpu.get(), Some(120));
assert_eq!(absent.get(), None);

state.widths().remove("cpu")?;

assert_eq!(cpu.get(), None);
assert!(cpu.set(80).is_err());
```
<!-- /shown -->

Ячейка записи пуста, пока её ключа нет, и удаление ключа опустошает её снова.
`set` по пустой получает отказ: ячейка — проекция записи, а вернуть ключ на
место может только карта.

## Что ячейка держит живым

Это ровно `Rc` и `Weak`, и не по аналогии: внутри и правда лежит
`Arc::downgrade` на поле.

`cell()` и `entry_cell()` представляют **проекцию** — это `Weak`. Источник
такая ячейка держит слабо, и `get` начнёт отвечать `None`, как только уйдёт
последний настоящий хендл на этот источник. Как и у `Weak`, ячейка при этом
жива, а вот `upgrade` изнутри уже не удаётся — отсюда и `None`, и
`WriteValue::SourceGone` на запись.

`into_cell()` и `into_entry_cell()` делают ячейку, которая источником
**владеет**, — это `Rc`. Хендл, который вы им передали, они забирают себе и
держат сильно. Заметьте: у структуры ничего не отнимают. Вытянуть хендл из
неё вообще нельзя — `state.sidebar_width()` и `state.widths()` отдают
`Arc::clone` того же самого, — так что ячейка становится ещё одним владельцем,
ровно как лишний `Rc::clone`. За ними и тянитесь там, где
переживает всех именно ячейка: она лежит в компоненте, попадает в `HashMap`,
уезжает в другой поток.

<!-- shown: a view, and a cell that owns what feeds it -->
```rust
let view = state.sidebar_width().cell();
let owned = state.sidebar_width().into_cell();

drop(state);
```
<!-- /shown -->

Обе вышли из одного поля, и живым его держит владеющая. Поэтому проекция выше
отвечает и после того, как структуру дропнули, а пустеет только тогда, когда
уйдёт и владеющая ячейка, — счётчик тот же самый, что у `Rc`.
