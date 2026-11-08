// Runs the web's shortcut resolution on key events into
// crates/t3-client/tests/fixtures/keybindings.json, which tests/keybindings.rs replays.
// From the repo root: node apps/rune/scripts/keybindings-reference.ts
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
const out = NodePath.resolve(
  import.meta.dirname,
  "../crates/t3-client/tests/fixtures/keybindings.json",
);
const webUrl = NodeURL.pathToFileURL(NodePath.resolve(repoRoot, "apps/web/package.json")).href;
const webSrc = NodeURL.pathToFileURL(NodePath.resolve(repoRoot, "apps/web/src")).href;
const scriptsUrl = NodeURL.pathToFileURL(import.meta.dirname).href;

// apps/rune has no node_modules: packages resolve as apps/web's do, and the web's own modules
// leave off extensions, as Vite allows.
NodeModule.registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL ?? "";
    if (parent.startsWith(scriptsUrl) && !specifier.startsWith(".") && !specifier.includes(":")) {
      return next(specifier, { ...context, parentURL: webUrl });
    }
    if (
      parent.startsWith(webSrc) &&
      specifier.startsWith(".") &&
      !/\.[cm]?[jt]sx?$/.test(specifier)
    ) {
      try {
        return next(`${specifier}.ts`, context);
      } catch {}
    }
    return next(specifier, context);
  },
  // Vite's import.meta.env doesn't exist in Node; the web's feature flags read it.
  load(url, context, next) {
    const loaded = next(url, context);
    if (!url.startsWith(webSrc) || loaded.source == null) return loaded;
    const source = String(loaded.source).replaceAll("import.meta.env", "({})");
    return { ...loaded, source };
  },
});

const { DEFAULT_RESOLVED_KEYBINDINGS, compileResolvedKeybindingRule } =
  await import("@t3tools/shared/keybindings");
const { resolveShortcutCommand } = await import(`${webSrc}/keybindings.ts`);

// A user's keybindings.json lands after the defaults, so it wins.
const overrides = [
  { key: "mod+shift+p", command: "commandPalette.toggle" },
  { key: "alt+j", command: "thread.next", when: "!composerFocus" },
].map((rule) => compileResolvedKeybindingRule(rule));
const rules = [...DEFAULT_RESOLVED_KEYBINDINGS, ...overrides];

type Ev = {
  key: string;
  code?: string;
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
  meta?: boolean;
};
const ev = (key: string, code: string | undefined, mods: string): Ev => ({
  key,
  ...(code ? { code } : {}),
  ctrl: mods.includes("c"),
  shift: mods.includes("s"),
  alt: mods.includes("a"),
  meta: mods.includes("m"),
});
const composer = { composerFocus: true, editableFocus: true };
const idle = {};

const cases: Array<[string, Ev, Record<string, boolean>]> = [
  ["palette", ev("k", "KeyK", "c"), idle],
  ["palette from the composer", ev("k", "KeyK", "c"), composer],
  ["palette, user binding", ev("P", "KeyP", "cs"), idle],
  ["file picker", ev("p", "KeyP", "c"), idle],
  ["diff", ev("d", "KeyD", "c"), idle],
  ["diff in a terminal splits", ev("d", "KeyD", "c"), { terminalFocus: true }],
  ["dictation", ev("D", "KeyD", "cs"), idle],
  ["sidebar", ev("b", "KeyB", "c"), idle],
  ["right panel", ev("b", "KeyB", "ca"), idle],
  ["new chat", ev("n", "KeyN", "c"), idle],
  ["previous thread", ev("{", "BracketLeft", "cs"), idle],
  ["next thread", ev("}", "BracketRight", "cs"), idle],
  ["next thread, user binding", ev("j", "KeyJ", "a"), idle],
  ["user binding needs its when", ev("j", "KeyJ", "a"), composer],
  ["back", ev("[", "BracketLeft", "c"), idle],
  ["find", ev("f", "KeyF", "c"), idle],
  ["model picker", ev("M", "KeyM", "cs"), idle],
  ["plan mode", ev("A", "KeyA", "cs"), idle],
  ["steer queued", ev("Enter", "Enter", "cs"), idle],
  ["edit queued", ev("ArrowUp", "ArrowUp", "a"), composer],
  ["edit queued needs the composer", ev("ArrowUp", "ArrowUp", "a"), idle],
  ["send alternate while running", ev("Enter", "Enter", "c"), { ...composer, turnRunning: true }],
  [
    "send background on a draft",
    ev("Enter", "Enter", "c"),
    { ...composer, draftThreadRoute: true },
  ],
  ["dvorak k by layout", ev("k", "KeyT", "c"), idle],
  ["comma by code", ev("<", "Comma", "cs"), idle],
  ["meta isn't ctrl", ev("k", "KeyK", "m"), idle],
  ["plain letter", ev("k", "KeyK", ""), idle],
  ["escape", ev("Escape", "Escape", ""), idle],
  ["f6", ev("F6", "F6", ""), idle],
];

const fixture = {
  rules,
  cases: cases.map(([name, e, context]) => ({
    name,
    event: e,
    context,
    command:
      resolveShortcutCommand(
        {
          key: e.key,
          code: e.code,
          ctrlKey: !!e.ctrl,
          shiftKey: !!e.shift,
          altKey: !!e.alt,
          metaKey: !!e.meta,
        },
        rules,
        { platform: "Linux x86_64", context: { editableFocus: false, ...context } },
      ) ?? null,
  })),
};

NodeFS.mkdirSync(NodePath.dirname(out), { recursive: true });
NodeFS.writeFileSync(out, `${JSON.stringify(fixture, null, 2)}\n`);
console.log(`wrote ${out}: ${rules.length} rules, ${fixture.cases.length} cases`);
