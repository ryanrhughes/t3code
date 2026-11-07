//! The guided generator: a full palette from a canvas and an accent seed
//! (`createVividThemeColors` in apps/web/src/themePalette.ts).

use crate::color::{
    Direction, Oklch, Rgb, canonical_color, contrast, from_oklch, from_rgb, is_dark, luminance,
    mix, oklch_to_rgb, parse_rgb, rgb_to_oklch, solve_lightness,
};
use crate::{Appearance, Role, Srgb, ThemeColors, default_colors};

const LIGHT_FOREGROUND: Rgb = Rgb::new(255.0, 250.0, 255.0);
const DARK_FOREGROUND: Rgb = Rgb::new(36.0, 21.0, 35.0);
const WHITE: Rgb = Rgb::new(255.0, 255.0, 255.0);
const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);
const STANDARD_LIGHT_MUTED_CONTRAST: f64 = 4.705;
const STANDARD_DARK_MUTED_CONTRAST: f64 = 5.082;

/// Builds every role from `canvas` and `accent` (any CSS color [`crate::parse_css_color`]
/// accepts), starting from the default palette for `appearance`. Dark or light derivation follows
/// the canvas luminance, not `appearance`.
pub fn vivid_theme_colors(appearance: Appearance, canvas: &str, accent: &str) -> ThemeColors {
    let canvas_fallback = match appearance {
        Appearance::Dark => Rgb::new(24.0, 15.0, 27.0),
        Appearance::Light => Rgb::new(250.0, 245.0, 250.0),
    };
    let canvas_rgb = parse_rgb(canvas, canvas_fallback);
    let accent_rgb = parse_rgb(accent, Rgb::new(168.0, 67.0, 112.0));
    let canvas = rgb_to_oklch(canvas_rgb);
    let accent = rgb_to_oklch(accent_rgb);
    let dark = is_dark(canvas_rgb);
    let hue = if accent.c < 0.02 { canvas.h } else { accent.h };
    let tint_c = (accent.c * 0.22).max(0.008).min(0.045);
    let step = if dark { 1.0 } else { -1.0 };
    let toward = if dark {
        Direction::Lighter
    } else {
        Direction::Darker
    };
    let pick = |dark_value: f64, light_value: f64| if dark { dark_value } else { light_value };

    let surface_at = |delta_l: f64, chroma: f64| Oklch {
        l: (canvas.l + step * delta_l).max(0.05).min(0.98),
        c: chroma,
        h: hue,
    };
    let tinted = |delta_l: f64| surface_at(delta_l, tint_c);
    let rgb_of = |color: Oklch| oklch_to_rgb(color, true);

    let text_base = Oklch {
        l: pick(0.95, 0.2),
        c: (accent.c * 0.25).min(0.035),
        h: hue,
    };
    let text_rgb = rgb_of(solve_lightness(text_base, &[canvas_rgb], 7.0, toward));
    let text_muted_rgb = standard_muted_text(canvas_rgb, text_rgb);

    let action = Oklch {
        l: (accent.l + pick(0.06, -0.02)).max(0.35).min(0.85),
        c: (accent.c * 0.9).max(0.06),
        h: (hue + 50.0) % 360.0,
    };
    let action_rgb = rgb_of(action);

    let sidebar = surface_at(0.045, tint_c * 1.4);
    let surface_raised = tinted(0.05);
    let secondary = surface_at(pick(0.1, 0.06), (accent.c * 0.5).min(0.09));
    let muted = surface_at(pick(0.06, 0.04), (accent.c * 0.35).min(0.06));
    let accent_surface = surface_at(pick(0.13, 0.08), (accent.c * 0.55).min(0.11));
    let message_surface = surface_at(pick(0.16, 0.1), (accent.c * 0.6).min(0.13));
    let update_surface = surface_at(pick(0.14, 0.09), (accent.c * 0.55).min(0.12));
    let search_match = rgb_of(surface_at(0.22, accent.c.min(0.12)));
    let search_match_active = rgb_of(surface_at(0.4, accent.c.min(0.18)));

    let foreground_on =
        |surface: Oklch| from_oklch(solve_lightness(text_base, &[rgb_of(surface)], 4.6, toward));
    let text = from_rgb(text_rgb);
    let text_muted = from_rgb(text_muted_rgb);
    let accent_color = from_rgb(accent_rgb);
    let canvas_color = from_rgb(canvas_rgb);
    let status = standard_status_colors(canvas_rgb);

    let mut colors = default_colors(appearance).clone();
    let roles: [(Role, Srgb); 61] = [
        (Role::Canvas, canvas_color),
        (Role::Chrome, canvas_color),
        (Role::Toolbar, canvas_color),
        (Role::ToolbarForeground, text),
        (
            Role::ToolbarBorder,
            from_oklch(surface_at(pick(0.14, 0.1), (accent.c * 0.4).min(0.08))),
        ),
        (
            Role::ToolbarControl,
            from_oklch(surface_at(pick(0.09, 0.05), tint_c * 1.3)),
        ),
        (Role::ToolbarControlForeground, text),
        (
            Role::ToolbarControlHover,
            from_oklch(surface_at(pick(0.14, 0.09), tint_c * 1.6)),
        ),
        (Role::Surface, from_oklch(tinted(0.015))),
        (Role::SurfaceRaised, from_oklch(surface_raised)),
        (Role::SurfaceOverlay, from_oklch(tinted(0.075))),
        (Role::Text, text),
        (Role::TextMuted, text_muted),
        (
            Role::Border,
            from_oklch(surface_at(pick(0.16, 0.12), (accent.c * 0.35).min(0.07))),
        ),
        (
            Role::Input,
            from_oklch(surface_at(pick(0.21, 0.16), (accent.c * 0.4).min(0.08))),
        ),
        (Role::Focus, accent_color),
        (Role::Accent, accent_color),
        (
            Role::AccentForeground,
            from_rgb(readable_foreground(accent_rgb)),
        ),
        (Role::Secondary, from_oklch(secondary)),
        (Role::SecondaryForeground, foreground_on(secondary)),
        (Role::Muted, from_oklch(muted)),
        (
            Role::MutedForeground,
            from_rgb(readable_text(rgb_of(muted), text_rgb, 1.0, 4.6)),
        ),
        (
            Role::Placeholder,
            from_rgb(readable_text(rgb_of(surface_raised), text_rgb, 1.0, 4.6)),
        ),
        (Role::SecondaryLabel, text_muted),
        (Role::IconMuted, text_muted),
        (Role::Error, status[0]),
        (Role::ErrorForeground, status[1]),
        (Role::ErrorSurface, status[2]),
        (Role::Warning, status[3]),
        (Role::WarningForeground, status[4]),
        (Role::WarningSurface, status[5]),
        (Role::Update, accent_color),
        (Role::UpdateForeground, foreground_on(update_surface)),
        (Role::UpdateSurface, from_oklch(update_surface)),
        (Role::AccentSurface, from_oklch(accent_surface)),
        (Role::AccentSurfaceForeground, foreground_on(accent_surface)),
        (Role::MessageSurface, from_oklch(message_surface)),
        (Role::MessageForeground, foreground_on(message_surface)),
        (Role::MessageAction, from_rgb(action_rgb)),
        (
            Role::MessageActionForeground,
            from_rgb(readable_foreground(action_rgb)),
        ),
        (
            Role::MessageActionHover,
            from_oklch(Oklch {
                l: action.l + pick(0.06, -0.06),
                ..action
            }),
        ),
        (
            Role::CodeBackground,
            from_oklch(surface_at(0.035, tint_c * 0.8)),
        ),
        (Role::CodeForeground, text),
        (Role::SearchMatchBackground, from_rgb(search_match)),
        (
            Role::SearchMatchForeground,
            from_rgb(readable_foreground(search_match)),
        ),
        (
            Role::SearchMatchActiveBackground,
            from_rgb(search_match_active),
        ),
        (
            Role::SearchMatchActiveForeground,
            from_rgb(readable_foreground(search_match_active)),
        ),
        (Role::Sidebar, from_oklch(sidebar)),
        (Role::SidebarForeground, foreground_on(sidebar)),
        (
            Role::SidebarMutedForeground,
            from_rgb(standard_muted_text(rgb_of(sidebar), text_rgb)),
        ),
        (
            Role::SidebarControlSurface,
            from_oklch(surface_at(pick(0.1, 0.07), tint_c * 1.5)),
        ),
        (
            Role::SidebarRowHover,
            from_oklch(surface_at(pick(0.08, 0.06), (accent.c * 0.45).min(0.08))),
        ),
        (
            Role::SidebarRowActive,
            from_oklch(surface_at(pick(0.12, 0.09), (accent.c * 0.55).min(0.1))),
        ),
        (
            Role::SidebarRowSelected,
            from_oklch(surface_at(pick(0.14, 0.1), (accent.c * 0.6).min(0.11))),
        ),
        (
            Role::SidebarBorder,
            from_oklch(surface_at(pick(0.17, 0.12), (accent.c * 0.4).min(0.08))),
        ),
        (Role::TerminalBackground, canvas_color),
        (Role::TerminalForeground, text),
        (Role::TerminalCursor, accent_color),
        (
            Role::TerminalSelection,
            from_oklch(surface_at(pick(0.18, 0.12), (accent.c * 0.55).min(0.12))),
        ),
        (
            Role::TerminalScrollbar,
            from_oklch(tinted(pick(0.22, 0.16))),
        ),
        (
            Role::TerminalScrollbarHover,
            from_oklch(tinted(pick(0.3, 0.22))),
        ),
    ];
    for (role, color) in roles {
        colors.set(role, color);
    }
    colors
}

/// error, errorForeground, errorSurface, warning, warningForeground, warningSurface.
fn standard_status_colors(canvas: Rgb) -> [Srgb; 6] {
    let dark = luminance(canvas) < 0.179;
    let (error, error_fg, warning, warning_fg) = if dark {
        ("#fb414a", "#ff6467", "#fe9a00", "#ffb900")
    } else {
        ("#fb2c36", "#c10007", "#fe9a00", "#bb4d00")
    };
    let surface_of = |value: &str| {
        mix(
            canvas,
            parse_rgb(value, canvas),
            if dark { 0.16 } else { 0.08 },
        )
    };
    let toward = if dark {
        Direction::Lighter
    } else {
        Direction::Darker
    };
    let readable_on = |foreground: &str, surface: Rgb| {
        from_oklch(solve_lightness(
            rgb_to_oklch(parse_rgb(foreground, canvas)),
            &[surface],
            4.6,
            toward,
        ))
    };
    let (error_surface, warning_surface) = (surface_of(error), surface_of(warning));
    let canonical = |value: &str| canonical_color(value).expect("status colors are literal hex");
    [
        canonical(error),
        readable_on(error_fg, error_surface),
        from_rgb(error_surface),
        canonical(warning),
        readable_on(warning_fg, warning_surface),
        from_rgb(warning_surface),
    ]
}

fn readable_foreground(background: Rgb) -> Rgb {
    let light = contrast(background, LIGHT_FOREGROUND);
    let dark = contrast(background, DARK_FOREGROUND);
    if light.max(dark) >= 4.5 {
        return if light >= dark {
            LIGHT_FOREGROUND
        } else {
            DARK_FOREGROUND
        };
    }
    if contrast(background, WHITE) >= contrast(background, BLACK) {
        WHITE
    } else {
        BLACK
    }
}

/// Mixes `foreground` toward `background` by up to `amount` while keeping `minimum_ratio`.
/// The result stays fractional, as the web's mix does.
fn readable_text(background: Rgb, foreground: Rgb, amount: f64, minimum_ratio: f64) -> Rgb {
    let softened = mix(foreground, background, amount);
    if contrast(softened, background) >= minimum_ratio {
        return softened;
    }
    let (mut readable, mut lower, mut upper) = (foreground, 0.0, amount);
    for _ in 0..12 {
        let candidate_amount = (lower + upper) / 2.0;
        let candidate = mix(foreground, background, candidate_amount);
        if contrast(candidate, background) >= minimum_ratio {
            readable = candidate;
            lower = candidate_amount;
        } else {
            upper = candidate_amount;
        }
    }
    readable
}

fn standard_muted_text(background: Rgb, foreground: Rgb) -> Rgb {
    let target = if is_dark(background) {
        STANDARD_DARK_MUTED_CONTRAST
    } else {
        STANDARD_LIGHT_MUTED_CONTRAST
    };
    readable_text(background, foreground, 1.0, target)
}
