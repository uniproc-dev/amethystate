import { joined, type Path } from "../src/path";
import type { Transport } from "../src/transport";

type Heard = (payload: unknown) => void;

export class FakeStore implements Transport {
  readonly values = new Map<string, unknown>();
  readonly watchers = new Map<string, Set<Heard>>();
  refuse = false;
  hold = false;
  #held: (() => void)[] = [];

  release(): void {
    for (const go of this.#held.splice(0)) go();
  }

  watching(path: Path): number {
    return this.watchers.get(joined(path))?.size ?? 0;
  }

  async scan(prefix: Path): Promise<Map<string, unknown>> {
    const under = new Map<string, unknown>();
    for (const [key, value] of this.values) {
      if (prefix.length === 0 || key === joined(prefix) || key.startsWith(`${joined(prefix)}.`)) {
        under.set(key, value);
      }
    }
    return under;
  }

  async set(path: Path, value: unknown, source: string | null): Promise<void> {
    await this.#answer();
    const key = joined(path);
    const old = this.values.get(key);
    this.values.set(key, value);
    this.#announce(path, value, old === undefined
      ? { type: "Insert", key: path.at(-1), value, source }
      : { type: "Update", key: path.at(-1), oldValue: old, newValue: value, source });
  }

  async remove(path: Path, source: string | null): Promise<void> {
    await this.#answer();
    const key = joined(path);
    const old = this.values.get(key);
    this.values.delete(key);
    this.#announce(path, undefined, { type: "Remove", key: path.at(-1), oldValue: old, source });
  }

  async clear(prefix: Path, source: string | null): Promise<void> {
    await this.#answer();
    for (const key of [...(await this.scan(prefix)).keys()]) this.values.delete(key);
    this.#tell(joined(prefix), { type: "Clear", source });
  }

  async flush(): Promise<void> {}

  watch(path: Path, heard: Heard): () => void {
    const key = joined(path);
    const set = this.watchers.get(key) ?? new Set<Heard>();
    set.add(heard);
    this.watchers.set(key, set);
    return () => {
      set.delete(heard);
      if (set.size === 0) this.watchers.delete(key);
    };
  }

  write(path: Path, value: unknown): void {
    void this.set(path, value, null);
  }

  delete(path: Path): void {
    void this.remove(path, null);
  }

  async #answer(): Promise<void> {
    if (this.hold) await new Promise<void>((go) => this.#held.push(go));
    if (this.refuse) throw new Error("refused");
  }

  #announce(path: Path, value: unknown, change: unknown): void {
    if (value !== undefined) this.#tell(joined(path), value);
    this.#tell(joined(path.slice(0, -1)), change);
  }

  #tell(key: string, payload: unknown): void {
    for (const heard of this.watchers.get(key) ?? []) heard(payload);
  }
}
