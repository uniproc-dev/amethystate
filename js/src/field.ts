import { joined, type Path } from "./path";
import type { Transport } from "./transport";
import { same } from "./same";

/** One stored value, kept in step with the store. */
export class Field<T> {
  #value: T;
  readonly #listeners = new Set<(value: T) => void>();
  readonly #stop: () => void;

  constructor(
    readonly path: Path,
    value: T,
    private readonly transport: Transport,
    private readonly source: string,
  ) {
    this.#value = value;
    this.#stop = transport.watch(path, (payload) => this.#take(payload as T));
  }

  /** What the field holds now. */
  get(): T {
    return this.#value;
  }

  /** Calls `listener` with the value now and on every change; the returned function stops it. */
  subscribe(listener: (value: T) => void): () => void {
    this.#listeners.add(listener);
    listener(this.#value);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** Takes `value` at once and writes it; a write the store refuses is taken back, and the promise rejects. */
  async set(value: T): Promise<void> {
    const before = this.#value;
    this.#take(value);

    try {
      await this.transport.set(this.path, value, this.source);
    } catch (why) {
      if (same(this.#value, value)) this.#take(before);
      throw why;
    }
  }

  /** Sets what `change` makes of the current value. */
  update(change: (value: T) => T): Promise<void> {
    return this.set(change(this.#value));
  }

  /** Stops watching the store; the field keeps the last value it held. */
  dispose(): void {
    this.#stop();
    this.#listeners.clear();
  }

  toString(): string {
    return `Field(${joined(this.path)})`;
  }

  #take(value: T): void {
    if (same(this.#value, value)) return;
    this.#value = value;
    for (const listener of [...this.#listeners]) listener(value);
  }
}
