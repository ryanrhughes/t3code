// Writes crates/t3-theme/assets/builtin-themes.json from the web's real palette code, so the
// Rust crate never parses CSS for built-ins. Run from the repo root after `vp i`:
//   node apps/rune/scripts/export-builtin-themes.ts
import * as NodeModule from "node:module";
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

export const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
export const crateRoot = NodePath.resolve(import.meta.dirname, "../crates/t3-theme");
const webPaletteUrl = NodeURL.pathToFileURL(
  NodePath.resolve(repoRoot, "apps/web/src/themePalette.ts"),
).href;
const scriptsUrl = NodeURL.pathToFileURL(import.meta.dirname).href;

// themePalette.ts reads environment themes through the app's atom registry, whose import graph
// reaches React. Only those two modules are replaced; every derivation function is the real one.
const STUBS: Record<string, string> = {
  "./rpc/atomRegistry":
    "export const appAtomRegistry = { get: () => globalThis.__t3EnvironmentThemes ?? [] };" +
    "export const subscribeAtomChanges = () => () => {};",
  "./state/server": "export const primaryServerEnvironmentThemesAtom = undefined;",
};

NodeModule.registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL ?? "";
    if (parent === webPaletteUrl && STUBS[specifier] !== undefined) {
      const source = encodeURIComponent(STUBS[specifier]);
      return { url: `data:text/javascript,${source}`, shortCircuit: true };
    }
    // The web uses bundler resolution: extensionless relative imports.
    if (
      specifier.startsWith(".") &&
      parent.startsWith("file:") &&
      !/\.[cm]?[jt]sx?$/.test(specifier)
    ) {
      const candidate = new URL(`${specifier}.ts`, parent);
      if (NodeFS.existsSync(NodeURL.fileURLToPath(candidate))) return next(candidate.href, context);
    }
    // apps/rune has no node_modules; resolve these scripts' packages as apps/web would.
    if (parent.startsWith(scriptsUrl) && !specifier.startsWith(".") && !specifier.includes(":")) {
      return next(specifier, { ...context, parentURL: webPaletteUrl });
    }
    return next(specifier, context);
  },
});

type Web = typeof import("../../web/src/themePalette.ts");
type Colors = Readonly<Record<string, string>>;

export async function loadWebThemePalette(): Promise<Web> {
  return import(webPaletteUrl);
}

/** Every role in THEME_COLOR_ROLES order, resolved to the hex the web paints. */
export function hexColors(web: Web, colors: Colors): Record<string, string> {
  return Object.fromEntries(
    web.THEME_COLOR_ROLES.map((role) => {
      const hex = web.themeColorToHex(colors[role] ?? "");
      if (!hex) throw new Error(`Unresolvable ${role}: ${colors[role]}`);
      return [role, hex];
    }),
  );
}

export function writeJson(path: string, value: unknown): void {
  NodeFS.mkdirSync(NodePath.dirname(path), { recursive: true });
  NodeFS.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
  console.log(`wrote ${path}`);
}

if (import.meta.main) {
  const web = await loadWebThemePalette();
  const { BUILT_IN_THEMES } = await import(
    NodeURL.pathToFileURL(NodePath.resolve(repoRoot, "packages/shared/src/themePalettes.ts")).href
  );
  writeJson(NodePath.resolve(crateRoot, "assets/builtin-themes.json"), {
    standard: {
      light: hexColors(web, web.getStandardPaintedThemeColors("light")),
      dark: hexColors(web, web.getStandardPaintedThemeColors("dark")),
    },
    themes: BUILT_IN_THEMES.map(
      (theme: import("../../web/src/themePalette.ts").ThemeDefinition) => ({
        id: theme.id,
        label: theme.label,
        appearance: theme.appearance,
        sidebarArtwork: theme.sidebarArtwork === true,
        colors: hexColors(web, theme.colors),
        variants: Object.fromEntries(
          Object.entries(theme.variants ?? {}).map(([appearance, colors]) => [
            appearance,
            hexColors(web, colors),
          ]),
        ),
      }),
    ),
  });
}
