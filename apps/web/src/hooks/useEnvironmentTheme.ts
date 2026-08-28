import type { EnvironmentTheme } from "@t3tools/contracts";
import { useAtomValue } from "@effect/atom-react";
import { useEffect, useSyncExternalStore } from "react";

import { primaryServerEnvironmentThemesAtom } from "../state/server";
import {
  THEME_COLOR_ROLES,
  createVividThemeColors,
  getDefaultThemeColors,
  getEnvironmentThemes,
  setEnvironmentThemes,
  subscribeToCustomThemes,
  toCanonicalThemeColor,
  type ThemeAppearance,
  type ThemeColorRole,
  type ThemeColors,
  type ThemeDefinition,
} from "../themePalette";
import { useTheme } from "./useTheme";

const THEME_COLOR_ROLE_SET: ReadonlySet<string> = new Set(THEME_COLOR_ROLES);

/**
 * Keeps only roles this build renders, so a machine publishing a role added by
 * a newer client degrades to ignoring it rather than to a broken palette.
 */
function publishedRoleOverrides(
  colors: Readonly<Record<string, string>> | undefined,
): Partial<Record<ThemeColorRole, string>> {
  if (colors === undefined) return {};
  const overrides: Partial<Record<ThemeColorRole, string>> = {};
  for (const [role, value] of Object.entries(colors)) {
    if (!THEME_COLOR_ROLE_SET.has(role)) continue;
    const normalized = toCanonicalThemeColor(value);
    if (normalized) overrides[role as ThemeColorRole] = normalized;
  }
  return overrides;
}

function publishedThemeColors(
  theme: EnvironmentTheme,
  appearance: ThemeAppearance,
  colors: Readonly<Record<string, string>> | undefined,
): ThemeColors {
  // Seeds generate the base with the guided theme editor\'s generator, so a
  // desktop theme arrives as a coherent T3 Code palette rather than a foreign
  // one. A standard exported theme file has no seeds; its colors stand on the
  // stock defaults exactly as an imported theme file\'s would.
  const base =
    theme.canvas !== undefined && theme.accent !== undefined
      ? createVividThemeColors(appearance, theme.canvas, theme.accent)
      : getDefaultThemeColors(appearance);
  return { ...base, ...publishedRoleOverrides(colors) };
}

/**
 * A published theme as the theme library renders it. Both published forms are
 * accepted: the seeded short form a desktop generates, and the standard
 * exported theme file the Download button produces — so any theme someone
 * shared can be dropped into the machine\'s themes directory as-is.
 */
export function environmentThemeDefinition(theme: EnvironmentTheme): ThemeDefinition {
  const variants: Partial<Record<ThemeAppearance, ThemeColors>> = {};
  for (const [variantAppearance, variantColors] of Object.entries(theme.variants ?? {})) {
    if (variantAppearance === theme.appearance) continue;
    variants[variantAppearance as ThemeAppearance] = publishedThemeColors(
      theme,
      variantAppearance as ThemeAppearance,
      variantColors,
    );
  }

  return {
    id: theme.id,
    label: theme.name,
    appearance: theme.appearance,
    colors: publishedThemeColors(theme, theme.appearance, theme.colors),
    ...(Object.keys(variants).length > 0 ? { variants } : {}),
    managed: true,
  };
}

/** The published themes as library entries; empty while none are published. */
export function useEnvironmentThemeDefinitions(): ReadonlyArray<ThemeDefinition> {
  return useSyncExternalStore(subscribeToCustomThemes, getEnvironmentThemes, () => []);
}

/**
 * Keeps the machine\'s published themes in the theme library for as long as
 * the primary environment publishes them. A client with a published theme
 * selected retints the moment the machine rewrites it; everyone else just
 * gains cards in the theme library.
 */
export function useEnvironmentThemeSync(): void {
  const published = useAtomValue(primaryServerEnvironmentThemesAtom);
  const { refreshTheme } = useTheme();

  useEffect(() => {
    setEnvironmentThemes(published.map(environmentThemeDefinition));
    // The palette is painted from a snapshot taken when the theme last
    // changed, so new colors only land if the active theme is re-applied.
    refreshTheme();
  }, [published, refreshTheme]);
}
