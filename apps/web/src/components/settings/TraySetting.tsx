import { useState } from "react";

import { Switch } from "../ui/switch";
import { SettingsRow } from "./settingsLayout";
import { searchableSetting } from "./settingsSearch";

// The desktop app owns this setting and applies it immediately, so the switch
// only needs the value it started with plus its own changes.
export function TraySetting() {
  const bridge = window.desktopBridge;
  const [enabled, setEnabled] = useState(() => bridge?.getTrayEnabled?.() === true);
  const [error, setError] = useState<string | null>(null);
  const setTrayEnabled = bridge?.setTrayEnabled;
  if (!setTrayEnabled) return null;

  const applyChange = async (next: boolean) => {
    setEnabled(next);
    setError(null);
    try {
      await setTrayEnabled(next);
    } catch (cause) {
      setEnabled(!next);
      setError(cause instanceof Error ? cause.message : "Couldn't change this setting.");
    }
  };

  return (
    <SettingsRow
      {...searchableSetting("system-tray")}
      description={
        error ??
        "Show agent status in the system tray (menu bar on macOS) and keep T3 Code running after you close its window. Quit from the tray."
      }
      control={
        <Switch
          checked={enabled}
          onCheckedChange={(checked) => void applyChange(Boolean(checked))}
          aria-label="System tray"
        />
      }
    />
  );
}
