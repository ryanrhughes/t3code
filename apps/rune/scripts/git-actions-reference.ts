// Runs the web's git action rules (GitActionsControl.logic.ts) over a grid of branch states
// into crates/t3-client/tests/fixtures/git-actions.json, which tests/git_actions.rs replays.
// From the repo root: node apps/rune/scripts/git-actions-reference.ts
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
const out = NodePath.resolve(
  import.meta.dirname,
  "../crates/t3-client/tests/fixtures/git-actions.json",
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
  load(url, context, next) {
    const loaded = next(url, context);
    if (!url.startsWith(webSrc) || loaded.source == null) return loaded;
    const source = String(loaded.source).replaceAll("import.meta.env", "({})");
    return { ...loaded, source };
  },
});

const logic = await import(`${webSrc}/components/GitActionsControl.logic.ts`);

const statuses: Array<Record<string, unknown>> = [];
for (const provider of ["github", "gitlab", null, "sourcehut"])
  for (const refName of ["feature", null])
    for (const hasWorkingTreeChanges of [false, true])
      for (const hasUpstream of [false, true])
        for (const aheadCount of [0, 2])
          for (const behindCount of [0, 1])
            for (const aheadOfDefaultCount of [undefined, 0, 3])
              for (const pr of [null, "open", "merged"]) {
                // Other hosts only need the wording checked, not the whole grid again.
                if (provider !== "github" && (refName === null || behindCount > 0)) continue;
                // Only what the rules read, to keep the fixture small.
                statuses.push({
                  ...(provider === null ? {} : { sourceControlProvider: { kind: provider } }),
                  refName,
                  hasWorkingTreeChanges,
                  hasUpstream,
                  aheadCount,
                  behindCount,
                  ...(aheadOfDefaultCount === undefined ? {} : { aheadOfDefaultCount }),
                  pr: pr === null ? null : { state: pr },
                });
              }

const cases: Array<Record<string, unknown>> = [];
for (const [index, status] of statuses.entries())
  for (const isDefaultRef of [false, true])
    for (const hasPrimaryRemote of [true, false]) {
      const q = logic.resolveQuickAction(status, false, isDefaultRef, hasPrimaryRemote);
      cases.push({
        status: index,
        isDefaultRef,
        hasPrimaryRemote,
        quick: [q.label, q.disabled, q.kind, q.action ?? q.hint ?? null],
        // The menu doesn't depend on the default ref; once per status is enough.
        menu: isDefaultRef
          ? null
          : logic
              .buildMenuItems(status, false, hasPrimaryRemote)
              .map((m: { id: string; label: string; disabled: boolean }) => [
                m.id,
                m.label,
                m.disabled,
              ]),
      });
    }
// Busy disables everything, whatever the branch looks like.
const busy = statuses[statuses.length - 1];
const q = logic.resolveQuickAction(busy, true, false, true);
cases.push({
  status: statuses.length - 1,
  busy: true,
  isDefaultRef: false,
  hasPrimaryRemote: true,
  quick: [q.label, q.disabled, q.kind, q.action ?? q.hint ?? null],
  menu: logic
    .buildMenuItems(busy, true, true)
    .map((m: { id: string; label: string; disabled: boolean }) => [m.id, m.label, m.disabled]),
});

const confirm = ["commit", "push", "create_pr", "commit_push", "commit_push_pr"].flatMap((a) =>
  [false, true].map((d) => [a, d, logic.requiresDefaultBranchConfirmation(a, d)]),
);

// Results repeat a lot: each distinct one is stored once and cases point at it.
const results: unknown[] = [];
const seen = new Map<string, number>();
const intern = (value: unknown) => {
  const key = JSON.stringify(value);
  if (!seen.has(key)) seen.set(key, results.push(value) - 1);
  return seen.get(key);
};
for (const c of cases) {
  c.quick = intern(c.quick);
  if (c.menu !== null) c.menu = intern(c.menu);
}

NodeFS.writeFileSync(out, `${JSON.stringify({ statuses, results, cases, confirm })}\n`);
console.log(`wrote ${cases.length} cases to ${NodePath.relative(repoRoot, out)}`);
