---
title: Переход со своего решения
sidebar:
  order: 24
---

Что делать, решает один вопрос: лежат ли ваши настройки в файле TOML, JSON или
RON, где поля структуры записаны на верхнем уровне?

## Файл — это ваша структура

Структуру, которую serde читает из файла и пишет обратно:

<!-- shown: the struct serde was writing -->
```rust
#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    pub window: Window,
}
```
<!-- /shown -->

объявите ещё раз, с теми же полями, на верхнем уровне хранилища:

<!-- shown: the same fields at the top of the store -->
```rust
#[amethystate(mode = "persistent", as_root)]
pub struct Settings {
    #[amestate(default = "light".to_string())]
    pub theme: String,

    #[amestate(default = Window { width: 800, height: 600 })]
    pub window: Window,
}
```
<!-- /shown -->

Включите движок того формата, в котором записан файл:

```toml
amethystate = { version = "0.21", features = ["toml"] }
```

и откройте тот файл, что у приложения уже есть:

<!-- shown: opening the file that was there -->
```rust
let store = StoreBuilder::new(file).backend(Backend::Toml).build()?;
let settings = Settings::load_with(&store)?;
```
<!-- /shown -->

Преобразовывать ничего не нужно. `as_root` кладёт поля туда же, куда их клал
serde, поэтому `load_with` читает то, что лежит в файле, а поле, которого в файле
нет, получает значение по умолчанию. Рядом с файлом store держит свой `.meta`:
[Что лежит на диске](/amethystate/ru/store/files/).

После сохранения старая структура по-прежнему читает файл TOML или JSON, так что
его откроет и старая сборка приложения. С RON иначе: store записывает его картой,
`{ "theme": .. }`, а serde читает структуру только из `(theme: ..)`.

Если старая структура переименовывала поля через serde, здесь для этого свои
слова: [Что здесь значит serde](/amethystate/ru/state/serde/).

## Всё остальное

Своя база данных, формат не из этих трёх, файл, устроенный не так, как
структура, — открыть такое на месте store не может. Значения переносят один раз,
своим кодом, сразу после открытия: прочитайте старый источник тем, чем читаете
сейчас, запишите значения через структуру и удалите старое:

<!-- shown: moving the values in once -->
```rust
let mut settings = Settings::load_with(&store)?;

if let Some(old) = read_old_settings(&old_file)? {
    settings.mutate(|now| {
        now.theme = old.theme;
        now.window = old.window;
    })?;
    std::fs::remove_file(&old_file)?;
}
```
<!-- /shown -->

`mutate` возвращается, когда значения уже на диске, поэтому старый файл уходит
только после того, как они сохранены. Если процесс упадёт между этими двумя
шагами, останется и то и другое, и при следующем запуске перенесутся те же
значения.

Шагом миграции этого не сделать: шагу не над чем работать. Шаги переписывают то,
что в хранилище уже лежит, а в новом не выполняется ни один. См.
[Что происходит при чистой установке](/amethystate/ru/migrations/overview/#что-происходит-при-чистой-установке).
