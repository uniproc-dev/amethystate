import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { eventChannel, joined, type Path } from "./path";

/** How the primitives reach the store: the Tauri plugin, or a stand-in for one. */
export interface Transport {
  scan(prefix: Path): Promise<Map<string, unknown>>;
  set(path: Path, value: unknown, source: string | null): Promise<void>;
  remove(path: Path, source: string | null): Promise<void>;
  clear(prefix: Path, source: string | null): Promise<void>;
  flush(prefix: Path): Promise<void>;
  watch(path: Path, heard: (payload: unknown) => void): () => void;
}

const command = (name: string) => `plugin:amethystate|amethystate_${name}`;

/** The store behind the `amethystate` Tauri plugin. */
export function tauri(): Transport {
  return {
    async scan(prefix) {
      const values = await invoke<Record<string, unknown>>(command("get_prefix"), { prefix: joined(prefix) });
      return new Map(Object.entries(values));
    },

    async set(path, value, source) {
      await invoke(command("set"), { key: joined(path), value, source });
    },

    async remove(path, source) {
      await invoke(command("delete"), { key: joined(path), source });
    },

    async clear(prefix, source) {
      await invoke(command("delete_prefix"), { prefix: joined(prefix), source });
    },

    async flush(prefix) {
      await invoke(command("flush"), { prefix: joined(prefix) });
    },

    watch(path, heard) {
      const key = joined(path);
      let stopped = false;
      let unlisten: (() => void) | null = null;

      const started = invoke(command("subscribe"), { key })
        .then(() => listen(eventChannel(path), (event) => heard(event.payload)))
        .then((stop) => {
          if (stopped) stop();
          else unlisten = stop;
        })
        .catch((why) => console.error(`amethystate: ${key} could not be watched`, why));

      return () => {
        if (stopped) return;
        stopped = true;
        unlisten?.();
        void started.finally(() => invoke(command("unsubscribe"), { key }));
      };
    },
  };
}
