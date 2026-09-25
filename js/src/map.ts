import { joined, type Path } from "./path";
import type { Transport } from "./transport";

/** One change to a map, as the store announces it. */
export type MapChange<V> =
  | { type: "Insert"; key: string; value: V; source: string | null }
  | { type: "Update"; key: string; oldValue: V; newValue: V; source: string | null }
  | { type: "Remove"; key: string; oldValue: V; source: string | null }
  | { type: "Clear"; source: string | null };

export type Entries<V> = ReadonlyArray<readonly [string, V]>;

/** A map stored one entry per level under its path, kept in step with the store. */
export class ReactiveMap<V> {
  readonly #held = new Map<string, V>();
  #entries: Entries<V> | null = null;
  readonly #listeners = new Set<(entries: Entries<V>) => void>();
  readonly #keyListeners = new Map<string, Set<(value: V | undefined) => void>>();
  readonly #changeListeners = new Set<(change: MapChange<V>) => void>();
  readonly #stop: () => void;

  constructor(
    readonly path: Path,
    entries: Iterable<readonly [string, V]>,
    private readonly transport: Transport,
    private readonly source: string,
  ) {
    for (const [key, value] of entries) this.#held.set(key, value);
    this.#stop = transport.watch(path, (payload) => {
      const change = payload as MapChange<V>;
      if (change.source !== this.source) this.#apply(change);
    });
  }

  get(key: string): V | undefined {
    return this.#held.get(key);
  }

  has(key: string): boolean {
    return this.#held.has(key);
  }

  get size(): number {
    return this.#held.size;
  }

  /** Every entry in the order of its key; the same array until something changes. */
  entries(): Entries<V> {
    this.#entries ??= [...this.#held].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return this.#entries;
  }

  /** Calls `listener` with the entries now and after every change; the returned function stops it. */
  subscribe(listener: (entries: Entries<V>) => void): () => void {
    this.#listeners.add(listener);
    listener(this.entries());
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** Calls `listener` with the value under `key` now and whenever it changes or goes. */
  subscribeKey(key: string, listener: (value: V | undefined) => void): () => void {
    const listeners = this.#keyListeners.get(key) ?? new Set();
    listeners.add(listener);
    this.#keyListeners.set(key, listeners);
    listener(this.get(key));
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) this.#keyListeners.delete(key);
    };
  }

  /** Calls `listener` with each change as it happens, and nothing now. */
  onChange(listener: (change: MapChange<V>) => void): () => void {
    this.#changeListeners.add(listener);
    return () => {
      this.#changeListeners.delete(listener);
    };
  }

  /** Puts `value` under `key`, whether or not something was there. */
  insert(key: string, value: V): Promise<void> {
    const old = this.#held.get(key);
    const change: MapChange<V> = this.#held.has(key)
      ? { type: "Update", key, oldValue: old as V, newValue: value, source: this.source }
      : { type: "Insert", key, value, source: this.source };
    return this.#write(change, () => this.transport.set([...this.path, key], value, this.source));
  }

  /** Replaces the value under a `key` the map has; one it does not have is refused. */
  update(key: string, value: V): Promise<void> {
    if (!this.#held.has(key)) {
      return Promise.reject(new Error(`${joined([...this.path, key])} is not in the map`));
    }
    return this.insert(key, value);
  }

  /** Removes `key`; a key the map does not have is left alone. */
  remove(key: string): Promise<void> {
    if (!this.#held.has(key)) return Promise.resolve();
    const change: MapChange<V> = { type: "Remove", key, oldValue: this.#held.get(key) as V, source: this.source };
    return this.#write(change, () => this.transport.remove([...this.path, key], this.source));
  }

  /** Removes every entry. */
  clear(): Promise<void> {
    return this.#write({ type: "Clear", source: this.source }, () => this.transport.clear(this.path, this.source));
  }

  /** Stops watching the store; the map keeps the entries it held. */
  dispose(): void {
    this.#stop();
    this.#listeners.clear();
    this.#keyListeners.clear();
    this.#changeListeners.clear();
  }

  async #write(change: MapChange<V>, send: () => Promise<void>): Promise<void> {
    const before = [...this.#held];
    this.#apply(change);

    try {
      await send();
    } catch (why) {
      for (const undo of this.#undoing(change, new Map(before))) this.#apply(undo);
      throw why;
    }
  }

  #undoing(change: MapChange<V>, before: Map<string, V>): MapChange<V>[] {
    const source = this.source;
    if (change.type === "Clear") {
      return [...before].map(([key, value]) => ({ type: "Insert", key, value, source }));
    }

    const key = change.key;
    const was = before.get(key);
    const had = before.has(key);
    const now = this.#held.get(key);
    const has = this.#held.has(key);

    if (had && has) return [{ type: "Update", key, oldValue: now as V, newValue: was as V, source }];
    if (!had && has) return [{ type: "Remove", key, oldValue: now as V, source }];
    if (had && !has) return [{ type: "Insert", key, value: was as V, source }];
    return [];
  }

  #apply(change: MapChange<V>): void {
    const touched: string[] = [];

    switch (change.type) {
      case "Insert":
        this.#held.set(change.key, change.value);
        touched.push(change.key);
        break;
      case "Update":
        this.#held.set(change.key, change.newValue);
        touched.push(change.key);
        break;
      case "Remove":
        this.#held.delete(change.key);
        touched.push(change.key);
        break;
      case "Clear":
        touched.push(...this.#held.keys(), ...this.#keyListeners.keys());
        this.#held.clear();
        break;
    }

    this.#entries = null;
    for (const key of new Set(touched)) {
      for (const listener of [...(this.#keyListeners.get(key) ?? [])]) listener(this.get(key));
    }
    for (const listener of [...this.#changeListeners]) listener(change);
    const entries = this.entries();
    for (const listener of [...this.#listeners]) listener(entries);
  }
}
