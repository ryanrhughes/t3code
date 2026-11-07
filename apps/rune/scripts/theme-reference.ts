// Runs the web's real theme derivation and writes crates/t3-theme/tests/fixtures/reference.json,
// which the Rust parity tests replay. Run from the repo root after `vp i`:
//   node apps/rune/scripts/theme-reference.ts
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";
import {
  crateRoot,
  hexColors,
  loadWebThemePalette,
  repoRoot,
  writeJson,
} from "./export-builtin-themes.ts";

const web = await loadWebThemePalette();
const Schema = await import("effect/Schema");
const Option = await import("effect/Option");
const { EnvironmentTheme } = await import(
  NodeURL.pathToFileURL(NodePath.resolve(repoRoot, "packages/contracts/src/server.ts")).href
);
const decodeEnvironmentTheme = Schema.decodeUnknownOption(EnvironmentTheme);

const omarchy = {
  // The server names published themes by file; the wire carries the id.
  id: "omarchy",
  ...JSON.parse(
    // What Omarchy's theme-set hook writes to <T3 home>/userdata/themes/omarchy.json (Tokyo
    // Night), kept here so the fixture is the same on every machine.
    NodeFS.readFileSync(NodePath.resolve(import.meta.dirname, "fixtures/omarchy.json"), "utf8"),
  ),
};

const environmentInputs: ReadonlyArray<[string, unknown]> = [
  ["omarchy", omarchy],
  [
    "light-seeds-with-overrides",
    {
      id: "paper",
      name: "  Paper  ",
      appearance: "light",
      canvas: "#fdf6ec",
      accent: "#c2571b",
      colors: {
        text: "#222",
        bogus: "#ffffff",
        border: "not-a-color",
        focus: "rgb(10 20 30 / 50%)",
        surface: "hsl(30 50% 90%)",
        input: "rgba(200, 100, 50, 0.25)",
        sidebar: "oklch(62% 0.2 280deg / 50%)",
        muted: "#abcd",
        secondary: "hsla(120, 40%, 60%, 80%)",
        messageAction: "oklch(0.7 0.37 150)",
      },
      variants: { dark: { canvas: "#101010", accent: "oklch(0.7 0.15 40)", text: "white" } },
    },
  ],
  [
    "dark-seeds-only",
    { id: "midnight", name: "Midnight", appearance: "dark", canvas: "#101a2c", accent: "#4f8fe8" },
  ],
  [
    "dark-gray-low-chroma",
    { id: "graphite", name: "Graphite", appearance: "dark", canvas: "#2b2b2b", accent: "#8a8a8a" },
  ],
  [
    "light-low-chroma-short-hex",
    { id: "fog", name: "Fog", appearance: "light", canvas: "#eee", accent: "#7a7f86" },
  ],
  [
    "colors-only",
    {
      id: "plain",
      name: "Plain",
      appearance: "dark",
      colors: { canvas: "#000", accent: "oklch(62% 0.2 280deg)", sidebar: "oklab(0.6 0.1 -0.1)" },
      variants: { light: { canvas: "white", text: "transparent" } },
    },
  ],
  [
    "variant-only",
    {
      id: "variant-only",
      name: "Variant only",
      appearance: "light",
      variants: { dark: { accent: "#ff0000" } },
    },
  ],
  [
    "no-usable-colors",
    { id: "empty", name: "Empty", appearance: "dark", colors: { canvas: "nope", nope: "#fff" } },
  ],
  [
    "reserved-built-in-id",
    { id: "grove", name: "Grove", appearance: "dark", canvas: "#000000", accent: "#ffffff" },
  ],
  [
    "reserved-legacy-id",
    { id: "t3-iris", name: "Iris", appearance: "dark", canvas: "#000000", accent: "#ffffff" },
  ],
  [
    "bad-id",
    { id: "Bad Id", name: "Bad", appearance: "dark", canvas: "#000000", accent: "#ffffff" },
  ],
  [
    "seed-without-hash",
    { id: "nohash", name: "No hash", appearance: "dark", canvas: "000000", accent: "#ffffff" },
  ],
  [
    "blank-name",
    { id: "blank", name: "   ", appearance: "dark", canvas: "#000000", accent: "#ffffff" },
  ],
  [
    "bad-appearance",
    { id: "dim", name: "Dim", appearance: "dim", canvas: "#000000", accent: "#ffffff" },
  ],
  [
    "bad-role-key",
    {
      id: "badkey",
      name: "Bad key",
      appearance: "dark",
      colors: { "9text": "#fff", text: "#fff" },
    },
  ],
];

function definitionJson(definition: import("../../web/src/themePalette.ts").ThemeDefinition) {
  return {
    id: definition.id,
    label: definition.label,
    appearance: definition.appearance,
    colors: hexColors(web, definition.colors),
    variants: Object.fromEntries(
      Object.entries(definition.variants ?? {}).map(([appearance, colors]) => [
        appearance,
        hexColors(web, colors),
      ]),
    ),
  };
}

const decodedThemes: Array<import("@t3tools/contracts").EnvironmentTheme> = [];
const environment = environmentInputs.map(([label, theme]) => {
  const decoded = Option.getOrNull(decodeEnvironmentTheme(theme));
  const definition = decoded ? web.publishedThemeDefinitions([decoded])[0] : undefined;
  if (decoded && definition) decodedThemes.push(decoded);
  return { label, theme, definition: definition ? definitionJson(definition) : null };
});

// The vivid seeds from apps/web/src/themePalette.test.ts.
const vividSeeds: ReadonlyArray<["light" | "dark", string, string]> = [
  ["light", "#f4f9f2", "#1d8a4e"],
  ["dark", "#101a2c", "#4f8fe8"],
  ["dark", "#211a23", "#df5398"],
  ["light", "#fdf6ec", "#c2571b"],
  ["light", "#111827", "#8ab4f8"],
  ["dark", "#f5ecf5", "#a84370"],
];
const vivid = vividSeeds.map(([appearance, canvas, accent]) => ({
  appearance,
  canvas,
  accent,
  colors: hexColors(web, web.createVividThemeColors(appearance, canvas, accent)),
}));

const parseInputs = [
  "#abc",
  "#abcd",
  "#a1b2c3",
  "#a1b2c380",
  "A1B2C3",
  "  #FFF  ",
  "black",
  "White",
  "transparent",
  "rgb(10 20 30)",
  "rgb(10 20 30 / 50%)",
  "rgb(10% 20% 30%)",
  "rgba(10, 20, 30, 0.5)",
  "rgb(10,20,30)",
  "rgb(300 -5 20)",
  "rgb(none 20 30)",
  "rgb(10 20 30 / none)",
  "hsl(350 80% 50%)",
  "hsl(350deg 80% 50% / 0.3)",
  "hsla(120, 40%, 60%, 80%)",
  "hsl(0.5turn 100% 50%)",
  "hsl(200 60 40)",
  "oklch(0.62 0.2 280)",
  "oklch(62% 0.2 280deg / 50%)",
  "oklch(0.7 0.37 150)",
  "oklch(1.2 50% 30)",
  "oklch(0.5 0 none)",
  "oklab(0.6 0.1 -0.1)",
  "oklab(60% 25% -25% / 0.4)",
  "oklch(.5 .1 1e2)",
  "rgb(10 20)",
  "rgb(10, 20 30)",
  "hsl(10% 20% 30%)",
  "oklch(0.5 0.1 10%)",
  "nope",
  "",
  "#abcde",
];
const parse = parseInputs.map((input) => ({ input, hex: web.themeColorToHex(input) }));

(globalThis as { __t3EnvironmentThemes?: unknown }).__t3EnvironmentThemes =
  web.publishedThemeDefinitions(decodedThemes);

type Halves = { light?: string; dark?: string };
const resolveCases: Array<{
  theme: string;
  mode: "light" | "dark" | "system";
  halves: Halves;
  systemDark: boolean;
}> = [];
const themeIds = [
  "system",
  "light",
  "dark",
  "t3-code",
  "unknown-theme",
  "t3-chat-dark",
  "t3-grove",
  "omarchy",
  "paper",
  "plain",
  "variant-only",
];
for (const { id } of [
  web.T3_CHAT_THEME,
  web.GROVE_THEME,
  web.OCEAN_THEME,
  web.EMBER_THEME,
  web.IRIS_THEME,
]) {
  themeIds.push(id);
}
for (const theme of new Set(themeIds)) {
  for (const mode of ["light", "dark", "system"] as const) {
    for (const systemDark of mode === "system" ? [false, true] : [false]) {
      resolveCases.push({ theme, mode, halves: {}, systemDark });
    }
  }
}
resolveCases.push(
  { theme: "grove", mode: "system", halves: { dark: "omarchy" }, systemDark: true },
  { theme: "grove", mode: "system", halves: { dark: "omarchy" }, systemDark: false },
  { theme: "omarchy", mode: "light", halves: { light: "t3-iris" }, systemDark: false },
  // A half that cannot paint its appearance is ignored, as parseThemeHalves drops it.
  {
    theme: "ocean",
    mode: "light",
    halves: { light: "omarchy", dark: "missing" },
    systemDark: false,
  },
  { theme: "system", mode: "dark", halves: { dark: "ember" }, systemDark: false },
);

const resolved = resolveCases.map(({ theme, mode, halves, systemDark }) => {
  const parsedHalves = web.parseThemeHalves(JSON.stringify(halves));
  const appearance = web.resolveThemeAppearance(
    theme,
    systemDark,
    mode === "system",
    mode,
    parsedHalves,
  );
  const half = web.resolveThemeHalf(theme, parsedHalves, appearance);
  // Mirrors applyThemePalette: a palette paints its variables; otherwise index.css paints the standard theme.
  const palette = web.getThemeDefinition(half);
  const colors = palette
    ? (web.getThemeColorsForMode(palette, appearance) ?? palette.colors)
    : web.getStandardPaintedThemeColors(appearance);
  return {
    preference: { theme, mode, halves },
    systemDark,
    expected: {
      id: palette?.id ?? null,
      appearance,
      // useEnvironmentIdentificationMode: artwork is allowed unless a palette forbids it.
      sidebarArtwork: palette === null || web.themeAllowsSidebarArtwork(half),
      colors: hexColors(web, colors),
    },
  };
});

writeJson(NodePath.resolve(crateRoot, "tests/fixtures/reference.json"), {
  environment,
  vivid,
  parse,
  resolve: resolved,
});
