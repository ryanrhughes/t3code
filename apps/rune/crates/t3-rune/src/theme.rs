//! T3 palettes (`t3-theme`) onto Rune: a rune-kit `Kit` for the components plus the T3 roles the
//! app's own views paint with. Built when the theme inputs change, never per frame.
use rune::Color;
use rune_kit::{Kit, PaletteSeed};
use t3_theme::{Appearance, ResolvedTheme, Role, Srgb, ThemeDefinition, ThemePreference};

/// What views read: the kit, and the T3 roles kit tokens don't cover.
#[derive(Clone)]
pub struct Look {
    pub kit: Kit,
    pub id: String,
    pub dark: bool,
    pub canvas: Color,
    pub sidebar: Color,
    pub sidebar_fg: Color,
    pub sidebar_muted: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub surface: Color,
    pub raised: Color,
    pub message: Color,
    pub message_fg: Color,
    pub code_bg: Color,
    pub code_fg: Color,
    pub accent: Color,
    pub action: Color,
    pub action_fg: Color,
    pub error_fg: Color,
    pub error_surface: Color,
    pub warning_fg: Color,
    pub warning_surface: Color,
}

/// Rune colours are linear; T3 roles are gamma-encoded sRGB.
fn lin(c: Srgb) -> Color {
    let d = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    Color {
        r: d(c.r),
        g: d(c.g),
        b: d(c.b),
        a: c.a,
    }
}

pub fn look(t: &ResolvedTheme) -> Look {
    let r = |role: Role| lin(t.colors.get(role));
    let dark = t.appearance == Appearance::Dark;
    let mut kit = Kit::from_palette(PaletteSeed {
        bg: r(Role::Canvas),
        fg: r(Role::Text),
        accent: r(Role::Accent),
    });
    let p = &mut kit.c;
    p.page = r(Role::Canvas);
    p.bg_base = r(Role::Canvas);
    // SplitView paints the window (sidebar and frame) with bg_deep.
    p.bg_deep = r(Role::Sidebar);
    p.layer_01 = r(Role::Surface);
    p.layer_02 = r(Role::SurfaceRaised);
    p.layer_03 = r(Role::SurfaceOverlay);
    p.surface_raised = r(Role::SurfaceRaised);
    p.surface_float = r(Role::SurfaceOverlay);
    p.text_base = r(Role::Text);
    p.text_strong = r(Role::Text);
    p.text_muted = r(Role::MutedForeground);
    p.text_faint = r(Role::Placeholder);
    p.icon_base = r(Role::Text);
    p.icon_muted = r(Role::IconMuted);
    p.icon_faint = r(Role::Placeholder);
    p.border_base = r(Role::Border);
    p.border_muted = r(Role::Border);
    p.border_weak = r(Role::Border);
    p.border_strong = r(Role::Input);
    p.border_focus = r(Role::Focus);
    p.text_accent = r(Role::Accent);
    p.icon_accent = r(Role::Accent);
    p.bg_accent = r(Role::MessageAction);
    p.danger.bg = r(Role::ErrorSurface);
    p.danger.fg = r(Role::ErrorForeground);
    p.danger.icon = r(Role::Error);
    p.warning.bg = r(Role::WarningSurface);
    p.warning.fg = r(Role::WarningForeground);
    p.warning.icon = r(Role::Warning);
    Look {
        kit,
        id: t.id.clone(),
        dark,
        canvas: r(Role::Canvas),
        sidebar: r(Role::Sidebar),
        sidebar_fg: r(Role::SidebarForeground),
        sidebar_muted: r(Role::SidebarMutedForeground),
        text: r(Role::Text),
        muted: r(Role::MutedForeground),
        border: r(Role::Border),
        surface: r(Role::Surface),
        raised: r(Role::SurfaceRaised),
        message: r(Role::MessageSurface),
        message_fg: r(Role::MessageForeground),
        code_bg: r(Role::CodeBackground),
        code_fg: r(Role::CodeForeground),
        accent: r(Role::Accent),
        action: r(Role::MessageAction),
        action_fg: r(Role::MessageActionForeground),
        error_fg: r(Role::ErrorForeground),
        error_surface: r(Role::ErrorSurface),
        warning_fg: r(Role::WarningForeground),
        warning_surface: r(Role::WarningSurface),
    }
}

/// Both appearances, so a system light/dark flip needs no rebuild.
pub struct Looks {
    pub light: Look,
    pub dark: Look,
}

impl Looks {
    pub fn build(pref: &ThemePreference, env: &[ThemeDefinition]) -> Looks {
        Looks {
            light: look(&t3_theme::resolve(pref, env, false)),
            dark: look(&t3_theme::resolve(pref, env, true)),
        }
    }

    pub fn get(&self, system_dark: bool) -> &Look {
        if system_dark { &self.dark } else { &self.light }
    }
}

/// Theme choices for the picker: (id, label), built-ins then environment themes.
pub fn choices(env: &[ThemeDefinition]) -> Vec<(String, String)> {
    let mut v = vec![("system".to_owned(), "T3 Code".to_owned())];
    v.extend(
        t3_theme::builtin_themes()
            .iter()
            .chain(env)
            .map(|t| (t.id.clone(), t.label.clone())),
    );
    v
}
