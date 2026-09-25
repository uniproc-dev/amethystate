import { expect, test } from "vitest";
import spelled from "../../crates/core/amethystate-core/tests/fixtures/paths.json";
import { eventChannel, joined, split } from "../src/path";

test.each(spelled)("$joined is spelled as the store spells it", (one) => {
  expect(joined(one.levels)).toBe(one.joined);
  expect(eventChannel(one.levels)).toBe(one.channel);
  expect(split(one.joined)).toEqual(one.levels);
});
