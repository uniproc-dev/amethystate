import { expect, test } from "vitest";
import { Loaded } from "../src/slice";
import { FakeStore } from "./fake";

function stocked() {
  const store = new FakeStore();
  store.values.set("todos.next_id", 4);
  store.values.set("todos.items.1", "a");
  store.values.set("todos.items.2", "b");
  store.values.set("todos.items.2.stray", "not an entry");
  store.values.set("todos.lists.1", "home");
  return store;
}

test("a field starts from what the store holds", async () => {
  const loaded = await Loaded.under(stocked(), ["todos"]);

  expect(loaded.field<number>(["todos", "next_id"]).get()).toBe(4);
});

test("a map takes the level under it and nothing deeper", async () => {
  const loaded = await Loaded.under(stocked(), ["todos"]);

  expect(loaded.map<string>(["todos", "items"]).entries()).toEqual([["1", "a"], ["2", "b"]]);
});

test("a field the store does not hold is refused by name", async () => {
  const loaded = await Loaded.under(stocked(), ["todos"]);

  expect(() => loaded.field<boolean>(["todos", "hide_done"])).toThrow("todos.hide_done");
});

test("disposing what was loaded releases every watch it took", async () => {
  const store = stocked();
  const loaded = await Loaded.under(store, ["todos"]);
  loaded.field<number>(["todos", "next_id"]);
  loaded.map<string>(["todos", "items"]);

  loaded.dispose();

  expect(store.watchers.size).toBe(0);
});
