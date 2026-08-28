import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { environmentThemeDefinition } from "./useEnvironmentTheme";
import {
  getThemeDefinition,
  installCustomTheme,
  invalidateCustomThemes,
  setEnvironmentThemes,
  THEME_COLOR_ROLES,
} from "../themePalette";

const OMARCHY_THEME = {
  id: "omarchy",
  name: "Omarchy",
  appearance: "dark",
  canvas: "#1a1b26",
  accent: "#7aa2f7",
} as const;

afterEach(() => {
  setEnvironmentThemes([]);
});

describe("environment themes", () => {
  it("generates every role from the two published seeds", () => {
    const theme = environmentThemeDefinition(OMARCHY_THEME);

    expect(theme.id).toBe("omarchy");
    expect(theme.label).toBe("Omarchy");
    expect(theme.appearance).toBe("dark");
    for (const role of THEME_COLOR_ROLES) {
      expect(theme.colors[role], `missing ${role}`).toBeTruthy();
    }
  });

  it("layers published roles over the generated palette", () => {
    const generated = environmentThemeDefinition(OMARCHY_THEME);
    const overridden = environmentThemeDefinition({
      ...OMARCHY_THEME,
      colors: { terminalSelection: "#292e42", error: "#f7768e" },
    });

    expect(overridden.colors.terminalSelection).not.toBe(generated.colors.terminalSelection);
    expect(overridden.colors.error).not.toBe(generated.colors.error);
    // Roles the machine did not publish keep the generated value.
    expect(overridden.colors.sidebar).toBe(generated.colors.sidebar);
  });

  // The standard exported theme file — the Download button's output — is a
  // valid published theme, so any shared theme can be dropped into the
  // machine's themes directory as-is.
  it("renders an exported theme file on the stock defaults", () => {
    const theme = environmentThemeDefinition({
      id: "shared-light",
      version: 1,
      name: "Shared Light",
      appearance: "light",
      colors: { canvas: "oklch(0.95 0.01 250)", accent: "#1e66f5" },
      variants: { dark: { canvas: "#1a1b26" } },
    });

    expect(theme.label).toBe("Shared Light");
    expect(theme.colors.accent).toBeTruthy();
    expect(theme.variants?.dark?.canvas).toBeTruthy();
    for (const role of THEME_COLOR_ROLES) {
      expect(theme.colors[role], `missing ${role}`).toBeTruthy();
      expect(theme.variants?.dark?.[role], `missing dark ${role}`).toBeTruthy();
    }
  });

  // A machine may publish roles a newer client added; an older one has to
  // ignore them rather than render a broken palette.
  it("ignores published roles this build does not render", () => {
    const theme = environmentThemeDefinition({
      ...OMARCHY_THEME,
      colors: { notARole: "#ff0000", text: "#ffffff" },
    });

    expect(theme.colors).not.toHaveProperty("notARole");
    for (const role of THEME_COLOR_ROLES) {
      expect(theme.colors[role], `missing ${role}`).toBeTruthy();
    }
  });

  it("resolves published ids only while the machine publishes them", () => {
    expect(getThemeDefinition("omarchy")).toBe(null);

    setEnvironmentThemes([environmentThemeDefinition(OMARCHY_THEME)]);
    expect(getThemeDefinition("omarchy")?.label).toBe("Omarchy");

    // The palettes are never saved, so they have to disappear with the
    // machine that published them rather than linger as stale entries.
    setEnvironmentThemes([]);
    expect(getThemeDefinition("omarchy")).toBe(null);
  });

  // Published ids share one namespace with the user's saved themes, and the
  // user was here first: their theme keeps working even if the machine later
  // publishes under the same id.
  it("lets a theme the user saved win an id collision", () => {
    const store = new Map<string, string>();
    vi.stubGlobal("window", {
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      localStorage: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => store.set(key, value),
        removeItem: (key: string) => store.delete(key),
      },
    });
    invalidateCustomThemes();
    try {
      const environment = environmentThemeDefinition(OMARCHY_THEME);
      setEnvironmentThemes([environment]);
      installCustomTheme({ ...environment, label: "My Omarchy" });

      expect(getThemeDefinition("omarchy")?.label).toBe("My Omarchy");
    } finally {
      vi.unstubAllGlobals();
      invalidateCustomThemes();
    }
    expect(getThemeDefinition("omarchy")?.label).toBe("Omarchy");
  });
});
