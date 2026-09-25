/** Whether two stored values are the same value: they arrive as JSON, so a copy counts. */
export function same(a: unknown, b: unknown): boolean {
  return Object.is(a, b) || JSON.stringify(a) === JSON.stringify(b);
}
