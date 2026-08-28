import { useAtomValue } from "@effect/atom-react";
import { useEffect } from "react";

import { primaryServerDefaultThemeAtom } from "../state/server";
import { getThemeDefinition } from "../themePalette";
import { useEnvironmentThemeDefinitions } from "./useEnvironmentTheme";
import { useTheme } from "./useTheme";

const APPLIED_DEFAULT_THEME_STORAGE_KEY = "t3code:default-theme-applied:v1";

/**
 * Which environment default this client last applied, so `t3 theme set` acts
 * once per value: the client switches when the environment's theme is set,
 * and a theme the user picks afterwards sticks until the next set.
 */
function readAppliedDefaultTheme(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(APPLIED_DEFAULT_THEME_STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeAppliedDefaultTheme(themeId: string): void {
  try {
    window.localStorage.setItem(APPLIED_DEFAULT_THEME_STORAGE_KEY, themeId);
  } catch {
    // Unrecordable means the next config event applies again; harmless.
  }
}

/**
 * Applies the environment\'s theme (`t3 theme set <id>`). Setting it switches
 * this client — live when connected, on the next connect otherwise — and then
 * steps aside: a theme the user picks in Settings afterwards wins until the
 * environment\'s theme is set again. The environment\'s own published themes
 * are valid targets, which is why this waits for an id that does not resolve
 * yet — the default and the palette it names arrive independently.
 */
export function useDefaultThemeAdoption(): void {
  const defaultTheme = useAtomValue(primaryServerDefaultThemeAtom);
  const { setTheme } = useTheme();
  // Re-runs adoption when a late-arriving published theme makes the
  // requested id resolvable.
  const environmentThemes = useEnvironmentThemeDefinitions();

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (defaultTheme.length === 0) return;
    if (readAppliedDefaultTheme() === defaultTheme) return;
    if (getThemeDefinition(defaultTheme) === null) return;
    if (setTheme(defaultTheme)) writeAppliedDefaultTheme(defaultTheme);
  }, [defaultTheme, environmentThemes, setTheme]);
}
