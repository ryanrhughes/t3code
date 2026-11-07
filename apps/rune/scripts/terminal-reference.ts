// Runs packages/shared/src/terminalOutput.ts on a corpus of real-world command output and writes
// crates/t3-client/tests/fixtures/terminal.json, which tests/terminal.rs replays against the
// Rust port. Run from the repo root after `vp i`:
//   node apps/rune/scripts/terminal-reference.ts
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
const out = NodePath.resolve(
  import.meta.dirname,
  "../crates/t3-client/tests/fixtures/terminal.json",
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

const terminal = await import("@t3tools/shared/terminalOutput");

const E = "\u001b";
const lines = (n: number, width = 10) =>
  Array.from(
    { length: n },
    (_, i) => `line ${String(i).padStart(3, "0")} ${"x".repeat(width)}`,
  ).join("\n");

const cases: Array<{ name: string; chunks: Array<string>; max?: number }> = [
  { name: "plain", chunks: ["hello\nworld\n"] },
  { name: "crlf", chunks: ["a\r\nb\r\n"] },
  { name: "progress", chunks: ["Downloading 10%\rDownloading 50%", "\rDownloading 100%\n"] },
  { name: "colors", chunks: [`${E}[1;31merror${E}[0m: bad\n${E}[32mok${E}[39m done`] },
  {
    name: "extended-colors",
    chunks: [`${E}[38;5;208morange${E}[0m ${E}[38;2;10;20;30mrgb${E}[m ${E}[48;5;22mbg${E}[0m`],
  },
  { name: "colon-colors", chunks: [`${E}[38:2::255:0:0mred${E}[0m ${E}[4:3mcurly${E}[4:0m off`] },
  { name: "erase-line", chunks: [`line one${E}[2K\rreplaced\n`] },
  { name: "cursor-up-rewrite", chunks: ["a: 1\nb: 1\n", `${E}[2A${E}[2Ka: 2\n${E}[2Kb: 2\n`] },
  { name: "split-escape", chunks: [`${E}[3`, `1mred${E}`, `[0m tail`] },
  { name: "osc-title", chunks: [`${E}]0;title\u0007visible\n`] },
  { name: "osc-hyperlink", chunks: [`${E}]8;;http://x${E}\\link${E}]8;;${E}\\ after`] },
  { name: "tabs-backspace", chunks: ["a\tb\nabc\b\bX\n"] },
  { name: "clear-screen", chunks: ["old\n", `${E}[2Jnew`] },
  { name: "clear-scrollback", chunks: [`${lines(70)}\n${E}[3Jafter`] },
  { name: "save-restore", chunks: [`12345${E}[s${E}[3Dxx${E}[uY ${E}7ab${E}8Z`] },
  { name: "scroll-past-window", chunks: [`${lines(80)}\n`, `${E}[5Aup`] },
  { name: "position", chunks: [`abc\ndef${E}[1;2HZ${E}[5G!`] },
  {
    name: "inverse-badges",
    chunks: [`${E}[7minv${E}[0m ${E}[42mPASS${E}[0m ${E}[30;47mdark on light${E}[0m`],
  },
  {
    name: "bright",
    chunks: [`${E}[91mbright${E}[0m ${E}[103mbg${E}[0m ${E}[2;3;9mdim${E}[22;23;29m back`],
  },
  { name: "long-unfinished-escape", chunks: [`ok ${E}[${"1;".repeat(40)}`] },
  { name: "unicode", chunks: ["héllo → wörld\n"] },
  { name: "c1-and-del", chunks: ["a\u0085b\u007fc\u0000d\n"] },
  { name: "intermediates", chunks: [`${E}(Bplain ${E}#8x`] },
  { name: "tail-lines", chunks: [lines(50, 20)], max: 200 },
  { name: "tail-long-line", chunks: ["y".repeat(500)], max: 200 },
  { name: "tail-styled-cut", chunks: [`${E}[32m${"g".repeat(300)}${E}[0m`], max: 200 },
  {
    name: "tail-incremental",
    chunks: Array.from({ length: 30 }, (_, i) => `${E}[3${i % 8}mrow ${i}${E}[0m\n`),
    max: 120,
  },
  {
    name: "below-cursor-trim",
    chunks: [`${lines(10, 30)}\n`, `${E}[9A${"z".repeat(40)}`],
    max: 150,
  },
];

const fixture = cases.map(({ name, chunks, max = terminal.TERMINAL_OUTPUT_TAIL_CHARS }) => {
  let state = terminal.EMPTY_TERMINAL_OUTPUT;
  for (const chunk of chunks) state = terminal.appendTerminalOutput(state, chunk, max);
  const spans = terminal
    .terminalOutputSpans(state.text, terminal.TERMINAL_PALETTES.dark)
    .map((span) => [span.text, span.style]);
  return {
    name,
    chunks,
    max,
    text: state.text,
    truncated: state.truncated,
    pending: state.pending,
    spans,
  };
});

NodeFS.mkdirSync(NodePath.dirname(out), { recursive: true });
NodeFS.writeFileSync(out, `${JSON.stringify(fixture, null, 2)}\n`);
console.log(`wrote ${out}: ${fixture.length} cases`);
