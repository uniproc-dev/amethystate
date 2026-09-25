import { Todos } from "./bindings/amethystate";
import type { Todo } from "./bindings/Todo";

type View = { el: HTMLElement; dispose(): void };
type Page = { kind: "overview" } | { kind: "list"; id: string } | { kind: "settings" };
type Report = (why: unknown) => void;

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const el = Object.assign(document.createElement(tag), props);
  el.append(...children);
  return el;
}

function inOrder<V>(entries: ReadonlyArray<readonly [string, V]>): (readonly [string, V])[] {
  return [...entries].sort(([a], [b]) => Number(a) - Number(b));
}

function mint(todos: Todos, report: Report): string {
  const id = todos.nextId.get();
  todos.nextId.set(id + 1).catch(report);
  return String(id);
}

function draft(label: string, submit: (text: string) => void): HTMLElement {
  const input = h("input");
  const send = () => {
    const text = input.value.trim();
    if (text !== "") submit(text);
    input.value = "";
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") send();
  });
  return h("div", {}, input, " ", h("button", { onclick: send }, label));
}

function removeList(todos: Todos, list: string, report: Report): void {
  for (const [id, todo] of todos.items.entries()) {
    if (todo.list === list) todos.items.remove(id).catch(report);
  }
  todos.lists.remove(list).catch(report);
}

function overview(todos: Todos, go: (page: Page) => void, report: Report): View {
  const rows = h("div");
  const render = () => {
    rows.replaceChildren(
      ...inOrder(todos.lists.entries()).map(([id, list]) => {
        const mine = todos.items.entries().filter(([, todo]) => todo.list === id);
        const done = mine.filter(([, todo]) => todo.done).length;
        return h(
          "div",
          {},
          h("button", { onclick: () => go({ kind: "list", id }) }, list.name),
          ` ${done}/${mine.length} `,
          h("button", { onclick: () => removeList(todos, id, report) }, "✕"),
        );
      }),
    );
  };

  const stops = [todos.lists.subscribe(render), todos.items.subscribe(render)];
  const el = h(
    "div",
    {},
    h("h2", {}, "lists"),
    rows,
    draft("add list", (name) => {
      todos.lists.insert(mint(todos, report), { name }).catch(report);
    }),
  );
  return { el, dispose: () => stops.forEach((stop) => stop()) };
}

function row(todos: Todos, id: string, report: Report): View {
  const el = h("div");
  const stop = todos.items.subscribeKey(id, (todo) => {
    if (todo === undefined) {
      el.replaceChildren("(removed)");
      return;
    }
    const box = h("input", { type: "checkbox", checked: todo.done });
    box.addEventListener("change", () => {
      todos.items.update(id, { ...todo, done: !todo.done }).catch(report);
    });
    el.replaceChildren(
      h("label", {}, box, ` ${todo.title}`),
      " ",
      h("button", { onclick: () => todos.items.remove(id).catch(report) }, "✕"),
    );
  });
  return { el, dispose: stop };
}

function listPage(todos: Todos, list: string, report: Report): View {
  const el = h("div");
  const title = h("h2");
  const rows = h("div");
  const left = h("span");
  const mounted = new Map<string, View>();

  const body = h(
    "div",
    {},
    title,
    draft("add", (title) => {
      const todo: Todo = { list, title, done: false };
      todos.items.insert(mint(todos, report), todo).catch(report);
    }),
    rows,
    h("hr"),
    left,
    " ",
    h("button", {
      onclick: () => {
        for (const [id, todo] of todos.items.entries()) {
          if (todo.list === list && todo.done) todos.items.remove(id).catch(report);
        }
      },
    }, "clear done"),
  );

  const renderRows = () => {
    const hide = todos.hideDone.get();
    const mine = inOrder(todos.items.entries()).filter(([, todo]) => todo.list === list);
    const shown = mine.filter(([, todo]) => !(hide && todo.done)).map(([id]) => id);

    for (const [id, view] of mounted) {
      if (!shown.includes(id)) {
        view.dispose();
        mounted.delete(id);
      }
    }
    rows.replaceChildren(
      ...shown.map((id) => {
        const view = mounted.get(id) ?? row(todos, id, report);
        mounted.set(id, view);
        return view.el;
      }),
    );
    left.textContent = `${mine.filter(([, todo]) => !todo.done).length} left`;
  };

  const stops = [
    todos.lists.subscribeKey(list, (found) => {
      if (found === undefined) {
        el.replaceChildren(h("p", {}, "this list is gone"));
      } else {
        title.textContent = found.name;
        if (!el.contains(body)) el.replaceChildren(body);
      }
    }),
    todos.items.subscribe(renderRows),
    todos.hideDone.subscribe(renderRows),
  ];

  return {
    el,
    dispose: () => {
      stops.forEach((stop) => stop());
      mounted.forEach((view) => view.dispose());
    },
  };
}

function settings(todos: Todos, report: Report): View {
  const box = h("input", { type: "checkbox" });
  box.addEventListener("change", () => {
    todos.hideDone.set(box.checked).catch(report);
  });
  const stop = todos.hideDone.subscribe((hide) => {
    box.checked = hide;
  });
  const el = h("div", {}, h("h2", {}, "settings"), h("label", {}, box, " hide done"));
  return { el, dispose: stop };
}

async function start(root: HTMLElement): Promise<void> {
  const todos = await Todos.load();
  const failed = h("p", { className: "failed" });
  const report: Report = (why) => {
    failed.textContent = String(why);
  };
  const body = h("div");
  let current: View | null = null;

  const go = (page: Page) => {
    current?.dispose();
    current =
      page.kind === "overview"
        ? overview(todos, go, report)
        : page.kind === "list"
          ? listPage(todos, page.id, report)
          : settings(todos, report);
    body.replaceChildren(current.el);
  };

  root.replaceChildren(
    h(
      "nav",
      {},
      h("button", { onclick: () => go({ kind: "overview" }) }, "lists"),
      " ",
      h("button", { onclick: () => go({ kind: "settings" }) }, "settings"),
    ),
    h("hr"),
    body,
    failed,
  );
  go({ kind: "overview" });
}

const root = document.querySelector<HTMLElement>("#app");
if (root) start(root).catch((why) => (root.textContent = String(why)));
