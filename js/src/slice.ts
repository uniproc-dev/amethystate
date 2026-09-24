import { Field } from "./field";
import { ReactiveMap } from "./map";
import { joined, split, type Path } from "./path";
import type { Transport } from "./transport";

/** What a slice was loaded from: the store's values under its prefix, and the watches taken on them. */
export class Loaded {
  readonly #owned: { dispose(): void }[] = [];

  private constructor(
    readonly transport: Transport,
    readonly prefix: Path,
    readonly source: string,
    private readonly values: Map<string, unknown>,
  ) {}

  /** Reads everything the store holds under `prefix`. */
  static async under(transport: Transport, prefix: Path): Promise<Loaded> {
    return new Loaded(transport, prefix, crypto.randomUUID(), await transport.scan(prefix));
  }

  /** The field at `path`; one the store does not hold is refused, because its default lives on the Rust side. */
  field<T>(path: Path): Field<T> {
    const key = joined(path);
    if (!this.values.has(key)) {
      throw new Error(
        `${key} is not in the store: build the struct on the Rust side before the frontend loads it, so its defaults are written`,
      );
    }
    return this.#own(new Field<T>(path, this.values.get(key) as T, this.transport, this.source));
  }

  /** The map at `path`, with the entries one level under it. */
  map<V>(path: Path): ReactiveMap<V> {
    const entries: [string, V][] = [];
    for (const [key, value] of this.values) {
      const levels = split(key);
      if (levels.length === path.length + 1 && path.every((level, at) => levels[at] === level)) {
        entries.push([levels[path.length] as string, value as V]);
      }
    }
    return this.#own(new ReactiveMap<V>(path, entries, this.transport, this.source));
  }

  /** Stops every watch this load took. */
  dispose(): void {
    for (const owned of this.#owned.splice(0)) owned.dispose();
  }

  #own<T extends { dispose(): void }>(owned: T): T {
    this.#owned.push(owned);
    return owned;
  }
}

/** What a generated slice class is built on. */
export abstract class Slice {
  protected constructor(protected readonly loaded: Loaded) {}

  /** Writes whatever the store still buffers under this slice to disk. */
  save(): Promise<void> {
    return this.loaded.transport.flush(this.loaded.prefix);
  }

  /** Stops every watch the slice holds. */
  dispose(): void {
    this.loaded.dispose();
  }
}
