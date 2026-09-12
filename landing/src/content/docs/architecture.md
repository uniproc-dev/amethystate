---
title: Architecture
sidebar:
  order: 2
---

The same library from underneath. Nothing here is needed to use it — this is for
deciding whether it fits, reading a stack trace, or choosing an engine on
something other than taste.

## The parts, and what talks to what

<figure class="diagram">
<svg viewBox="0 0 960 620" role="img" aria-labelledby="parts-t">
<title id="parts-t">The store, with its migration engine, differ and reactive layer inside it; the disk above holding the data and the bookkeeping beside it; the debouncer between them; and the handles below.</title>
<defs>
<marker id="arw" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" fill="currentColor" opacity="0.5"/></marker>
<marker id="arw-lit" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L10 5 L0 10 z" fill="var(--sl-color-accent-high)"/></marker>
</defs>
<rect class="box" x="250" y="14" width="350" height="78" rx="16"/>
<text class="name" x="262" y="36">Disk</text>
<rect class="box" x="266" y="46" width="130" height="38" rx="19"/>
<text class="note" x="331" y="70" text-anchor="middle">bookkeeping</text>
<rect class="box" x="406" y="46" width="184" height="38" rx="19"/>
<text class="note" x="498" y="70" text-anchor="middle">the values</text>
<rect class="box" x="700" y="24" width="230" height="52" rx="26"/>
<text class="name" x="815" y="55" text-anchor="middle">Debouncer</text>
<rect class="box" x="24" y="188" width="132" height="62" rx="31"/>
<text class="name" x="90" y="225" text-anchor="middle">Migrations</text>
<rect class="box" x="190" y="130" width="690" height="270" rx="28"/>
<text class="name-big" x="856" y="164" text-anchor="end">STORE</text>
<rect class="box" x="214" y="192" width="256" height="62" rx="14"/>
<text class="name" x="342" y="229" text-anchor="middle">Migration engine</text>
<rect class="box" x="214" y="286" width="446" height="86" rx="14"/>
<text class="name" x="437" y="318" text-anchor="middle">Reactive layer</text>
<text class="note" x="437" y="342" text-anchor="middle">builds the handles · keeps the subscriptions · fires them</text>
<rect class="box" x="710" y="298" width="180" height="54" rx="27"/>
<text class="name" x="800" y="330" text-anchor="middle">Durable</text>
<rect class="group" x="230" y="496" width="600" height="88" rx="12"/>
<text class="note" x="230" y="486">handles</text>
<rect class="box" x="252" y="514" width="104" height="52" rx="26"/>
<text class="name" x="304" y="546" text-anchor="middle">Kv</text>
<rect class="box" x="388" y="514" width="104" height="52" rx="26"/>
<text class="name" x="440" y="546" text-anchor="middle">Field</text>
<rect class="box" x="524" y="514" width="104" height="52" rx="26"/>
<text class="name" x="576" y="546" text-anchor="middle">Cell</text>
<rect class="box" x="660" y="514" width="148" height="52" rx="26"/>
<text class="name" x="734" y="546" text-anchor="middle">Map</text>
<path class="edge" d="M360 540 L384 540" marker-end="url(#arw)" marker-start="url(#arw)"/>
<path class="edge" d="M496 540 L520 540" marker-end="url(#arw)" marker-start="url(#arw)"/>
<path class="edge" d="M632 540 L656 540" marker-end="url(#arw)" marker-start="url(#arw)"/>
<path class="edge" d="M156 221 L208 221" marker-end="url(#arw)"/>
<text class="note" x="90" y="272" text-anchor="middle">at open</text>
<path class="edge" d="M320 192 L320 98" marker-end="url(#arw)"/>
<text class="note" x="310" y="108" text-anchor="end">a migration,</text>
<text class="note" x="310" y="124" text-anchor="end">at once</text>
<path class="edge" d="M694 50 L608 50" marker-end="url(#arw)"/>
<text class="note" x="651" y="38" text-anchor="middle">after a pause</text>
<path class="edge" d="M740 130 L740 82" marker-end="url(#arw)"/>
<text class="note" x="750" y="112">schedule</text>
<path class="edge" d="M890 314 L916 314 L916 82" marker-end="url(#arw)" marker-start="url(#arw)"/>
<path class="edge" d="M500 92 L500 280" marker-end="url(#arw)"/>
<text class="note" x="490" y="108" text-anchor="end">an edit from outside</text>
<text class="note" x="490" y="124" text-anchor="end">text engines only</text>
<text class="note" x="512" y="212">read whole, then</text>
<text class="note" x="512" y="228">diffed against what</text>
<text class="note" x="512" y="244">the store holds</text>
<path class="edge-lit" d="M330 490 L330 378" marker-end="url(#arw-lit)"/>
<text class="note-lit" x="318" y="440" text-anchor="end">a write</text>
<path class="edge-lit" d="M540 372 L540 490" marker-end="url(#arw-lit)"/>
<text class="note-lit" x="552" y="440">an event</text>
<path class="edge" d="M790 490 L790 360" marker-end="url(#arw)" marker-start="url(#arw)"/>
<text class="note" x="802" y="432">a durable write,</text>
<text class="note" x="802" y="448">and its answer</text>
</svg>
<figcaption>

The reactive layer is the store's own code: it builds the handles, keeps the
list of who is watching what, and calls them back. It is not a tier between the
store and the handles, which is why the box is drawn inside.

The lit path is one write, and both halves of it happen before `set` returns.
The handle writes in; the subscription the layer registered when it built that
handle is what sets the handle's own value — with the store's stamp rather than
a fresh one, so two racing writes are ordered once and everyone sees the same
order. An edit that arrives from the file rather than from a caller comes back
along the very same edge.

`Durable` is not a component of its own — it is what any of the four handles
gets from `.durable()`. Both of its edges point both ways, and that is the whole
of what it does: it tells the debouncer not to wait the pause out, and it blocks
until the commit answers. The answer comes back the same way, to the handle that
asked. Every other write on this diagram returns before the disk has been
touched.

The edge coming back off the disk is a **text engine only**. Those three watch
their file because they are not its only writer, and what they read cannot go
straight to a handle: a file arrives whole, and which paths in it changed is
something that has to be worked out. That is the diff, and it is written on the
edge rather than drawn as a box because that is what it is — a pass over two
documents on the way in, not a part with a life of its own. What it works out is
fired through the very same subscription list a local write goes through, which
is why an application never learns where a change came from unless it asks. redb
and SQLite have no such edge; their file is theirs.

`Migrations` come in through the store's own API — `run_migrations`, which every
engine implements — but through a door of their own, and they are the one write
on this diagram that does not go through the debouncer. A step sees raw bytes
through the migration adapter, the whole pass runs in one transaction the engine
opens, and it lands when that transaction commits. There is nothing to debounce:
this happens while the store is opening, before a single handle exists, and the
next thing to run has to see it.

</figcaption>
</figure>

## A path is a list of levels

A path is levels, and only levels. `StorePath::segment("dark.mode")` is **one**
level whose name happens to contain a dot. The dotted spelling is how a path
writes itself down, not what it is: joining escapes a separator or a backslash
inside a name, and reading it back undoes that. Everything that compares,
orders or hashes a path does so by its levels, so the two spellings of one name
can never be mistaken for each other.

A path keeps whichever form it arrived in and works the other out only if
somebody asks. A declared path arrives from the macro with both halves already
written into the binary and checked against each other at compile time; a path
read back off a disk arrives in whichever form its engine holds, and a scan that
never needs the other half never pays for it.

The flat engines address by an encoding rather than by the spelling: each
level's bytes, each level terminated. Because the terminator cannot appear
inside a level, byte order over keys is level order and a subtree is exactly a
byte prefix — so a prefix scan is one range over the table with nothing to
filter afterwards. That is one decision, made once, and both flat engines get
their scans from it.

## Two families of engine

**redb and SQLite are flat.** One key-value space; a struct at `a.b` is one key
and `a.b.x` beside it is another. Values are msgpack on redb and JSON on SQLite.
Nothing has to be told which paths are structure and which are data, because
the key says so.

**JSON, TOML and RON are documents.** A document writes an object either way, so
it cannot tell "a struct at `a.b`" from "a level `a` holding a level `b`" — and
it has to, because a person opens this file. So the declarations decide: a
declared place, and every level on the way to one, is written as a nested tree
the way serde would have written it. Everything else — including a path *inside*
a declared value — goes in a plane of whole keys beside the tree, one level
named by the whole path. A map's entries nest; a key nobody declared does not.

The three text engines share one traversal and one diff. What differs between
them is only what a single node can hold, which is the format's business and
shows up as the limits each one has.

## A write goes down and comes back up

`field.set(x)` does not update the field and then persist it. It hands the value
down, and the field learns about it on the way back.

Interceptors run first and may turn the write down. Then the value is encoded
and compared against what is already stored: **identical bytes stop here** — no
event, no disk, no subscriber woken. Otherwise it enters the buffer, and the
store calls every matching subscriber **synchronously, on the thread that
wrote**, before scheduling anything. The field's own signal is set by that
callback, with the store's stamp rather than a fresh one, so two racing writes
are ordered once and everyone sees the same order.

This is why `set` then `get` returns the new value, why an unreadable value
leaves the old one in place and reports itself instead, and why an edit made
outside the process reaches subscribers by exactly the same path a local write
does — differing only in where it says it came from.

Writes reach disk after a quiet period. One thread waits the pause out and
flushes what accumulated; a burst becomes one flush. A flush that fails is
retried, and the retry budget bounds how long the store stays *quiet* about it,
not how long it keeps trying — it keeps trying until it lands or the store is
dropped. Outliving the budget reports once, hands back every path written since
the last flush that landed, and asks what writers should be told from then on.

Closing writes down whatever was still held. A write that cannot wait asks for
the disk directly — and what that buys differs by family: on a text engine one
commit covers the whole store, so everything buffered becomes durable with it;
on redb and SQLite it covers that write.

## The disk, and the second writer

A text store is two files — the data and what the library knows about it — each
replaced through a temporary file in the same directory, flushed, then renamed.
Each replacement is atomic on its own. The pair is not, and the order is chosen
so that the disagreement a crash can leave is the one that can be detected:
the bookkeeping claims more than the data holds, which the next open notices.

A text store also watches its file, because it is not the only writer. A touched
file is not a changed one, so the first thing a look does is hash the bytes and
compare them against what this store last wrote — no parse, no diff, and that is
the common case. Bytes that really differ are parsed, and if this store is
holding writes the file never got, they are laid back over what the file now
holds rather than overwriting it. What the file brought with it is diffed and
delivered as ordinary events.

## What sits beside the data

The version each prefix has reached, the shape each struct had when it last
wrote, the log of steps applied, which namespaces have had their defaults
written, and a record of how the bytes themselves were written.

That last one is the one that refuses. It names the codec, the key encoding and
the document layout, and an open that meets a value it does not write stops and
says which fact it stopped on. A name outside those namespaces is carried
through untouched, so an older build writing a store a newer one made does not
erase what it could not read.

The recorded shapes are compared by the places a declaration owns, never by the
name of the struct or the type. Renaming a type is not a change to the store,
and must not be read as one.

## Migrations are one transaction per pass

A pass starts at a prefix and opens one transaction covering it and everything
it reaches. Failure rolls that back and leaves the rest of the store alone, so
one prefix's bad step is something the report can name rather than something
that stops the open.

Nothing declares an order. A step that reads across into another prefix brings
that prefix up to date first, inside the same transaction — so the order is the
reaching, and a cycle is caught while it is happening, before anything has been
written that would need undoing.

## Who owns a path

Three mechanisms, and they answer different questions. What a constructor in
this process has already built refuses a second one and can say which instance
took it. What the binary *declares* refuses a raw write, and it knows about
structs nobody has built yet. What a previous run recorded on disk can only
tell — a claim by code that is not running protects nobody, and is there to
report drift rather than to forbid.

A fourth check sits outside all three, at read time: a map refusing an entry
more than one level below itself. It is deliberately not part of the tables,
because it is the only one that works against a writer no table knows about — a
raw write, a migration, or a person with an editor.

## What the shape costs

A path and a value is the whole data model: no queries, no indexes, no
transaction you can open yourself.

A process holds as many stores as it likes — that is ordinary, and the
per-instance bookkeeping exists for it. What the engines do not agree on is two
stores over **one file**: redb refuses the second outright, SQLite holds the
file, and the text engines let both in and reconcile at save time, which is what
the standoff above is for. Whether the two are in one process or two makes no
difference to any of them.

And a text engine's file is meant to be read and edited by a person, which is
why it costs a whole subsystem the flat engines do without.
