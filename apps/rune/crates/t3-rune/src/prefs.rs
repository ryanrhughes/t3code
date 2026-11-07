//! Per-device client preferences, as the web keeps in localStorage (`t3code:theme`,
//! `t3code:theme-appearance-mode`, `t3code:default-theme-applied:v2`).
use serde::{Deserialize, Serialize};
use t3_theme::{AppearanceMode, ThemePreference};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// `system | light | dark | <theme id>`.
    pub theme: String,
    /// `system | light | dark`.
    pub mode: String,
    /// The `(defaultTheme, defaultThemeSetAt)` generation already adopted.
    pub adopted_default_theme: Option<String>,
    /// What a message sent while an agent works does by default: `steer | queue`.
    pub follow_up: String,
    /// The sidebar's Settled shelf is open; closed by default, as the web's.
    pub settled_expanded: bool,
    pub snoozed_expanded: bool,
    /// Desktop notifications; off by default, as the web's `notificationMode`.
    pub notifications: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: "system".into(),
            mode: "system".into(),
            adopted_default_theme: None,
            follow_up: "steer".into(),
            settled_expanded: false,
            snoozed_expanded: false,
            notifications: false,
        }
    }
}

fn path() -> Option<std::path::PathBuf> {
    Some(t3_client::pairing::credential_path()?.with_file_name("prefs.json"))
}

impl Prefs {
    pub fn load() -> Prefs {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(p) = path() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_vec_pretty(self).unwrap_or_default());
    }

    pub fn theme(&self) -> ThemePreference {
        let mode = match self.mode.as_str() {
            "light" => AppearanceMode::Light,
            "dark" => AppearanceMode::Dark,
            _ => AppearanceMode::System,
        };
        ThemePreference {
            // An older build saved "" here from an empty server default.
            theme: if self.theme.trim().is_empty() {
                "system".into()
            } else {
                self.theme.clone()
            },
            mode,
            halves: Default::default(),
        }
    }

    pub fn set_theme(&mut self, id: &str) {
        self.theme = id.to_owned();
        // A named light/dark choice is also the appearance; a theme follows the system.
        self.mode = match id {
            "light" | "dark" => id.to_owned(),
            _ => "system".to_owned(),
        };
        self.save();
    }

    pub fn set_mode(&mut self, mode: &str) {
        self.mode = mode.to_owned();
        // "light" and "dark" name an appearance as well as a theme; keep them in step.
        if matches!(self.theme.as_str(), "light" | "dark") && mode != "system" {
            self.theme = mode.to_owned();
        }
        self.save();
    }

    pub fn toggle_settled(&mut self) {
        self.settled_expanded = !self.settled_expanded;
        self.save();
    }

    pub fn set_notifications(&mut self, on: bool) {
        self.notifications = on;
        self.save();
    }

    pub fn toggle_snoozed(&mut self) {
        self.snoozed_expanded = !self.snoozed_expanded;
        self.save();
    }

    pub fn set_follow_up(&mut self, behavior: &str) {
        self.follow_up = behavior.to_owned();
        self.save();
    }

    /// Adopts `t3 theme set` once per generation; returns true when it changed the theme.
    pub fn adopt_default_theme(&mut self, (theme, set_at): &(String, String)) -> bool {
        let generation = format!("{theme}@{set_at}");
        if self.adopted_default_theme.as_deref() == Some(generation.as_str()) {
            return false;
        }
        self.adopted_default_theme = Some(generation);
        self.set_theme(theme);
        true
    }
}
