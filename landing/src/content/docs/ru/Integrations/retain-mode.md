---
title: egui / iced / ratatui
---

У этих фреймворков явный цикл событий — либо перерисовка каждый кадр, либо проведение всех изменений через цикл сообщений. Естественного места для подписок нет, поэтому `amethystate` используют в режиме только сохранения: загрузить состояние при запуске, читать и менять поля напрямую, сбрасывать на диск, когда нужно.

## Схема

```rust
#[amethystate(prefix = "app", mode = "persistent")]
pub struct AppState {
    #[amestate(default = 800u32)]
    pub window_width: u32,

    #[amestate(default = "dark".to_string())]
    pub theme: String,
}
```

```rust
let mut state = AppState::load()?;

// read directly
println!("{}", state.theme);

// mutate and flush
state.mutate_lazy(|s| {
    s.theme = "light".to_string();
})?;
```

Внешние изменения — другой процесс, пишущий в то же хранилище, файл, отредактированный руками — в загруженной структуре не отразятся. Если это нужно, возьмите реактивный режим и вызывайте `.get()` в начале каждого кадра, чтобы опрашивать последнее значение.

## Примеры

- [`egui-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/egui-settings)
- [`iced-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/iced-settings)
- [`ratatui-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/ratatui-settings)
