// @vitest-environment happy-dom
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, test } from "vitest";
import { tauri } from "../src/transport";

type Call = [string, unknown];

function recording(answers: Record<string, unknown> = {}): Call[] {
  const calls: Call[] = [];
  mockIPC(
    (cmd, payload) => {
      if (!cmd.startsWith("plugin:event|")) calls.push([cmd, payload]);
      return answers[cmd];
    },
    { shouldMockEvents: true },
  );
  return calls;
}

const settled = () => new Promise((done) => setTimeout(done, 20));

afterEach(() => clearMocks());

test("writes name the joined path and who wrote", async () => {
  const calls = recording();
  const store = tauri();

  await store.set(["todos", "items", "a.b"], { done: true }, "me");
  await store.remove(["todos", "items", "3"], "me");
  await store.clear(["todos", "items"], null);

  expect(calls).toEqual([
    ["plugin:amethystate|amethystate_set", { key: "todos.items.a\\.b", value: { done: true }, source: "me" }],
    ["plugin:amethystate|amethystate_delete", { key: "todos.items.3", source: "me" }],
    ["plugin:amethystate|amethystate_delete_prefix", { prefix: "todos.items", source: null }],
  ]);
});

test("a scan hands back what the plugin answered", async () => {
  recording({ "plugin:amethystate|amethystate_get_prefix": { "todos.next_id": 3 } });

  const values = await tauri().scan(["todos"]);

  expect([...values]).toEqual([["todos.next_id", 3]]);
});

test("a watch subscribes, hears its channel, and lets go when stopped", async () => {
  const calls = recording();
  const heard: unknown[] = [];

  const stop = tauri().watch(["todos", "items"], (payload) => heard.push(payload));
  await settled();
  await emit("amethystate://todos:items", { type: "Clear", source: null });
  await emit("amethystate://todos:lists", { type: "Clear", source: null });
  stop();
  await settled();
  await emit("amethystate://todos:items", { type: "Clear", source: null });

  expect(heard).toEqual([{ type: "Clear", source: null }]);
  expect(calls).toEqual([
    ["plugin:amethystate|amethystate_subscribe", { key: "todos.items" }],
    ["plugin:amethystate|amethystate_unsubscribe", { key: "todos.items" }],
  ]);
});

test("a watch stopped before it started still lets go of the plugin", async () => {
  const calls = recording();

  const stop = tauri().watch(["todos", "items"], () => {});
  stop();
  await settled();

  expect(calls).toEqual([
    ["plugin:amethystate|amethystate_subscribe", { key: "todos.items" }],
    ["plugin:amethystate|amethystate_unsubscribe", { key: "todos.items" }],
  ]);
});
