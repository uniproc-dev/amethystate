import { expect, test } from "vitest";
import { Field } from "../src/field";
import { FakeStore } from "./fake";

const at = ["todos", "next_id"];

function watched(store: FakeStore) {
  store.values.set("todos.next_id", 1);
  const field = new Field<number>(at, 1, store, "me");
  const seen: number[] = [];
  field.subscribe((value) => seen.push(value));
  return { field, seen };
}

test("a subscriber is told the value it starts from", () => {
  const { seen } = watched(new FakeStore());

  expect(seen).toEqual([1]);
});

test("a set is seen before the store answers", async () => {
  const store = new FakeStore();
  const { field, seen } = watched(store);
  store.hold = true;

  const setting = field.set(9);

  expect(field.get()).toBe(9);
  expect(seen).toEqual([1, 9]);
  store.release();
  await setting;
  expect(seen).toEqual([1, 9]);
});

test("a refused set puts the old value back and says so", async () => {
  const store = new FakeStore();
  const { field, seen } = watched(store);
  store.refuse = true;

  await expect(field.set(9)).rejects.toThrow("refused");

  expect(field.get()).toBe(1);
  expect(seen).toEqual([1, 9, 1]);
});

test("a write made elsewhere reaches the field", () => {
  const store = new FakeStore();
  const { field, seen } = watched(store);

  store.write(at, 5);

  return Promise.resolve().then(() => {
    expect(field.get()).toBe(5);
    expect(seen).toEqual([1, 5]);
  });
});

test("an unsubscribed listener hears nothing more", async () => {
  const store = new FakeStore();
  store.values.set("todos.next_id", 1);
  const field = new Field<number>(at, 1, store, "me");
  const seen: number[] = [];
  const stop = field.subscribe((value) => seen.push(value));

  stop();
  await field.set(2);

  expect(seen).toEqual([1]);
});

test("a disposed field stops watching the store", () => {
  const store = new FakeStore();
  const field = new Field<number>(at, 1, store, "me");
  expect(store.watching(at)).toBe(1);

  field.dispose();

  expect(store.watching(at)).toBe(0);
});
