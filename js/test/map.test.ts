import { expect, test } from "vitest";
import { ReactiveMap } from "../src/map";
import { FakeStore } from "./fake";

const at = ["todos", "items"];

function watched(store: FakeStore, entries: [string, string][] = []) {
  for (const [key, value] of entries) store.values.set(`todos.items.${key}`, value);
  const map = new ReactiveMap<string>(at, entries, store, "me");
  const seen: string[] = [];
  map.subscribe((all) => seen.push(all.map(([key, value]) => `${key}=${value}`).join(",")));
  return { map, seen };
}

test("an insert is seen before the store answers", async () => {
  const store = new FakeStore();
  const { map, seen } = watched(store);
  store.hold = true;

  const inserting = map.insert("3", "milk");

  expect(map.get("3")).toBe("milk");
  expect(seen).toEqual(["", "3=milk"]);
  store.release();
  await inserting;
  expect(seen).toEqual(["", "3=milk"]);
});

test("a refused insert is taken back and says so", async () => {
  const store = new FakeStore();
  const { map, seen } = watched(store);
  store.refuse = true;

  await expect(map.insert("3", "milk")).rejects.toThrow("refused");

  expect(map.has("3")).toBe(false);
  expect(seen).toEqual(["", "3=milk", ""]);
});

test("a refused update puts the old value back", async () => {
  const store = new FakeStore();
  const { map } = watched(store, [["3", "milk"]]);
  store.refuse = true;

  await expect(map.update("3", "bread")).rejects.toThrow("refused");

  expect(map.get("3")).toBe("milk");
});

test("updating a key the map does not have is refused without touching it", async () => {
  const store = new FakeStore();
  const { map, seen } = watched(store);

  await expect(map.update("3", "bread")).rejects.toThrow("todos.items.3");

  expect(seen).toEqual([""]);
});

test("a refused clear brings every entry back", async () => {
  const store = new FakeStore();
  const { map } = watched(store, [["1", "a"], ["2", "b"]]);
  store.refuse = true;

  await expect(map.clear()).rejects.toThrow("refused");

  expect(map.entries()).toEqual([["1", "a"], ["2", "b"]]);
});

test("changes made elsewhere reach the map", async () => {
  const store = new FakeStore();
  const { map, seen } = watched(store, [["1", "a"]]);

  store.write([...at, "2"], "b");
  store.delete([...at, "1"]);
  await Promise.resolve();

  expect(map.entries()).toEqual([["2", "b"]]);
  expect(seen).toEqual(["1=a", "1=a,2=b", "2=b"]);
});

test("the store's echo of this map's own write is not heard twice", async () => {
  const store = new FakeStore();
  const { map, seen } = watched(store);

  await map.insert("3", "milk");

  expect(seen).toEqual(["", "3=milk"]);
});

test("a key's listener hears it go, whoever removes it", async () => {
  const store = new FakeStore();
  const { map } = watched(store, [["3", "milk"]]);
  const heard: (string | undefined)[] = [];
  map.subscribeKey("3", (value) => heard.push(value));

  store.delete([...at, "3"]);
  await Promise.resolve();

  expect(heard).toEqual(["milk", undefined]);
});

test("a key with a separator in it is written under one level", async () => {
  const store = new FakeStore();
  const { map } = watched(store);

  await map.insert("a.b", "dotted");

  expect([...store.values.keys()]).toEqual(["todos.items.a\\.b"]);
});

test("the entries a reader gets stay the same object until something changes", async () => {
  const store = new FakeStore();
  const { map } = watched(store, [["1", "a"]]);

  const first = map.entries();
  expect(map.entries()).toBe(first);

  await map.insert("2", "b");
  expect(map.entries()).not.toBe(first);
});

test("entries come in the order of their keys", () => {
  const { map } = watched(new FakeStore(), [["b", "2"], ["a", "1"], ["c", "3"]]);

  expect(map.entries().map(([key]) => key)).toEqual(["a", "b", "c"]);
});

test("a disposed map stops watching the store", () => {
  const store = new FakeStore();
  const { map } = watched(store);
  expect(store.watching(at)).toBe(1);

  map.dispose();

  expect(store.watching(at)).toBe(0);
});
