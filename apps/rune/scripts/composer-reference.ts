// Runs packages/shared/src/composerTrigger.ts on composer edge cases and writes
// crates/t3-client/tests/fixtures/composer.json for tests/composer.rs. Offsets are UTF-16, as
// JavaScript counts them. Run from the repo root after `vp i`:
//   node apps/rune/scripts/composer-reference.ts
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
const out = NodePath.resolve(
  import.meta.dirname,
  "../crates/t3-client/tests/fixtures/composer.json",
);
const serverUrl = NodeURL.pathToFileURL(
  NodePath.resolve(repoRoot, "apps/server/package.json"),
).href;
const scriptsUrl = NodeURL.pathToFileURL(import.meta.dirname).href;

// apps/rune has no node_modules; resolve packages as apps/server does.
NodeModule.registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL ?? "";
    if (parent.startsWith(scriptsUrl) && !specifier.startsWith(".") && !specifier.includes(":")) {
      return next(specifier, { ...context, parentURL: serverUrl });
    }
    return next(specifier, context);
  },
});

const { detectComposerTrigger, serializeComposerFileLink } =
  await import("@t3tools/shared/composerTrigger");

// [text, cursor]; a missing cursor is the end of the text.
const triggers: Array<[string, number?]> = [
  ["@"],
  ["look at @src/ma"],
  ["look at @src/ma rest", 15],
  ["/"],
  ["/rev"],
  ["/review now"],
  ["text\n/com"],
  ["/model"],
  ["/MODEL"],
  ["/model gpt"],
  ["/model  opus 4 "],
  ["/models"],
  ["/usr/bin"],
  ["/usr/bin", 2],
  ["run /usr/lo"],
  ["fix #12"],
  ["fix #"],
  ["fix #feat-x"],
  ["#!/bin"],
  ["use $deploy"],
  ["costs $5"],
  ["costs $10k."],
  ["costs $1.5e3"],
  ["€pdf"],
  ["£9.99"],
  ["$"],
  ["plain words"],
  ["email me@x.com"],
  ["a @b c", 4],
  ["tab\t@x"],
  ["ünïcödé @fïle"],
  ["héllo /cmd"],
  ["two\n@x\nthree", 6],
];

const paths = [
  "src/main.rs",
  "README.md",
  "dir with space/file name.txt",
  "src/a b(1).rs",
  "notes/#todo?.md",
  "weird/[brackets]\\back.rs",
  "unicode/ünï.ts",
  "deep/a/b/c/d.tsx",
  "100%/file.md",
];

const fixture = {
  triggers: triggers.map(([text, cursor]) => ({
    text,
    cursor: cursor ?? text.length,
    trigger: detectComposerTrigger(text, cursor ?? text.length),
  })),
  links: paths.map((path) => ({ path, link: serializeComposerFileLink(path) })),
};

NodeFS.mkdirSync(NodePath.dirname(out), { recursive: true });
NodeFS.writeFileSync(out, `${JSON.stringify(fixture, null, 2)}\n`);
console.log(`wrote ${out}: ${fixture.triggers.length} triggers, ${fixture.links.length} links`);
