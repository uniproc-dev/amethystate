---
title: Introduction
sidebar:
  order: 1
---

I don't believe in GUI in Rust. To anyone who wants to build an interface rather
than reinvent the wheel, I say: take Tauri or Electron.

I started with the question "what do I write a GUI in?" and, without much
deliberation, arrived at Slint. If all you need is to draw the interface, Rust is
a tolerable choice. But you outgrow that need quickly, and behind it stands a
scatter of problems that matter just as much: where the state lives, how the app
updates, and a dozen smaller things. Tauri has answers for them; the native
desktop has none at all.

This library is my answer, one of a dozen possible, to the question "how do I
keep the sidebar's width between launches?". I read the sources of Halloy,
Sniffnet and smaller projects. Each solves it on the spot, for its own needs, and
to my mind clumsily. So I decided to strain a little — that is, to strain the
GPUs on Anthropic's servers — and solve it more systematically.

## What it is

`amethystate` holds a Rust GUI application's state — reactive in memory, saved
on disk. You describe the state you want in one struct; saving it, making it
reactive, and migrating it when it changes are what the library is for.

```rust
#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
```

`state.port.set(9090)` returns to a caller who can already read `9090` back, and
to subscribers who have already heard about it. The disk catches up on its own.

Subscribers of `port`, and nobody else. What you watch is one field, one map
key, or one path — never the struct, and never the store. A write to `host`
reaches nothing that was listening to `port`, so a view redraws because the
thing it draws changed and not because something near it did.

## What it decides, so you do not

**Where the file goes.** A store opened by name lands where the platform keeps
application data, with the extension its engine wants.

**When to write.** Writes settle in memory and reach disk once per window,
counted from the first of them, so a slider dragged across its range costs a
flush every few hundred milliseconds rather than one per frame. Closing
the store writes down whatever it was still holding.

**What a value is on disk.** Five engines, three answers: redb keeps it as
MessagePack, SQLite as JSON, and the text engines write it into the document
itself, in that document's own syntax. The same declaration works against all
five, and what a given format cannot hold is reported rather than discovered
later.

**What to do when the struct changed.** The shape each struct had is recorded
beside the data. A version that went up runs the steps you declared; fields that
moved under an unchanged version are reported as drift, with a diff, and startup
carries on.

**What to do when somebody edited the file.** The text engines watch it. An edit
made outside arrives as the same events a write does, and a save that meets one
lays this store's own writes over what the file now holds rather than
overwriting it.

## Two kinds of state

They differ in one thing — whether a field is a handle — and share storage,
schema and migrations.

- **Reactive** — a field is a handle: you read it, write it, subscribe to it.
  The rest of the book assumes this one.
- **Persistent-only** — ordinary fields on an ordinary struct, saved when you
  say so. For frameworks that own their update loop, and for state nobody needs
  to watch. It does not see changes made elsewhere.

## What it is for, and what it is not

It is sized for state an application holds about itself: window layout, the
panel you had open, a filter, a recently-used list, a cache kept between runs.
Tens of keys is its smallest case and not its only one — a cache is thousands,
written in bursts and read in scans, and the library is built for that end too.

It is not a database. There are no queries, no indexes and no transactions you
can open; a path and a value is the whole data model. Open as many stores as you
have files to open — what the engines disagree about is two of them over the
*same* file, where redb refuses the second, SQLite holds the file, and the text
engines let both in and sort it out when they save.

## Where to go next

[Quick start](/amethystate/getting-started/quick-start/) is the shortest path
from nothing to a running store. [Architecture](/amethystate/architecture/) is
the same library from underneath: what a path is, what the two engine families
do differently, and what happens between a write and the disk.
