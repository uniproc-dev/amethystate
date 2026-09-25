/** Where a value lives in the store, as the levels it is under. */
export type Path = readonly string[];

const SEPARATOR = ".";
const ESCAPE = "\\";

/** The joined spelling the store keeps: levels between dots, a dot or backslash in a level escaped. */
export function joined(path: Path): string {
  return path
    .map((level) => level.replaceAll(ESCAPE, ESCAPE + ESCAPE).replaceAll(SEPARATOR, ESCAPE + SEPARATOR))
    .join(SEPARATOR);
}

/** The levels a joined spelling names. */
export function split(spelled: string): string[] {
  if (spelled === "") return [];

  const levels: string[] = [];
  let level = "";
  let escaped = false;
  for (const ch of spelled) {
    if (escaped) {
      level += ch;
      escaped = false;
    } else if (ch === ESCAPE) {
      escaped = true;
    } else if (ch === SEPARATOR) {
      levels.push(level);
      level = "";
    } else {
      level += ch;
    }
  }
  levels.push(level);
  return levels;
}

const kept = /[A-Za-z0-9-]/;
const bytes = new TextEncoder();

/** The Tauri event changes at `path` are announced on. */
export function eventChannel(path: Path): string {
  const levels = path.map((level) => {
    let name = "";
    for (const ch of level) {
      if (kept.test(ch)) {
        name += ch;
      } else {
        for (const byte of bytes.encode(ch)) name += `_${byte.toString(16).padStart(2, "0")}`;
      }
    }
    return name;
  });
  return `amethystate://${levels.join(":")}`;
}
