//! Port of "derives readable, distinctive vivid palettes from exact seeds" in
//! apps/web/src/themePalette.test.ts. Its `toMatch(/^oklch\(/)` checks have no equivalent: roles
//! here are already painted sRGB.

use t3_theme::{Appearance, Role, Srgb, parse_css_color, vivid_theme_colors};

/// WCAG contrast on 8-bit channels, as the web test's `contrastRatio` measures it.
fn contrast(first: Srgb, second: Srgb) -> f64 {
    let luminance = |color: Srgb| {
        let [r, g, b, _] = color.to_rgba8();
        [r, g, b]
            .map(|c| {
                let c = f64::from(c) / 255.0;
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            })
            .iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(c, w)| c * w)
            .sum::<f64>()
    };
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn derives_readable_distinctive_vivid_palettes_from_exact_seeds() {
    let seeds = [
        (Appearance::Light, "#f4f9f2", "#1d8a4e"),
        (Appearance::Dark, "#101a2c", "#4f8fe8"),
        (Appearance::Dark, "#211a23", "#df5398"),
        (Appearance::Light, "#fdf6ec", "#c2571b"),
        (Appearance::Light, "#111827", "#8ab4f8"),
        (Appearance::Dark, "#f5ecf5", "#a84370"),
    ];
    let white = parse_css_color("#ffffff").unwrap();
    for (appearance, canvas, accent) in seeds {
        let colors = vivid_theme_colors(appearance, canvas, accent);
        let c = |role: Role| colors.get(role);
        let ratio = |fg: Role, bg: Role| contrast(c(fg), c(bg));
        let seed = format!("{canvas}/{accent}");

        assert_eq!(c(Role::Canvas).to_hex(), canvas, "{seed}");
        assert_eq!(c(Role::Accent).to_hex(), accent, "{seed}");
        assert!(ratio(Role::Text, Role::Canvas) >= 7.0, "{seed}");
        assert!(ratio(Role::TextMuted, Role::Canvas) >= 4.5, "{seed}");
        assert!(ratio(Role::TextMuted, Role::Canvas) < 5.5, "{seed}");
        assert!(ratio(Role::MutedForeground, Role::Muted) >= 4.5, "{seed}");
        assert!(
            ratio(Role::MutedForeground, Role::Muted) < ratio(Role::Text, Role::Muted),
            "{seed}"
        );
        assert!(
            ratio(Role::Placeholder, Role::SurfaceRaised) >= 4.5,
            "{seed}"
        );
        assert!(
            ratio(Role::Placeholder, Role::SurfaceRaised) < ratio(Role::Text, Role::SurfaceRaised),
            "{seed}"
        );
        assert_eq!(c(Role::SecondaryLabel), c(Role::TextMuted), "{seed}");
        for (fg, bg) in [
            (Role::AccentForeground, Role::Accent),
            (Role::SearchMatchForeground, Role::SearchMatchBackground),
            (
                Role::SearchMatchActiveForeground,
                Role::SearchMatchActiveBackground,
            ),
            (Role::MessageActionForeground, Role::MessageAction),
            (Role::MessageForeground, Role::MessageSurface),
            (Role::SecondaryForeground, Role::Secondary),
            (Role::SidebarForeground, Role::Sidebar),
            (Role::ErrorForeground, Role::ErrorSurface),
        ] {
            assert!(
                ratio(fg, bg) >= 4.5,
                "{seed} {} on {}",
                fg.name(),
                bg.name()
            );
        }
        assert_ne!(c(Role::MessageAction), c(Role::Accent), "{seed}");
        assert_eq!(c(Role::Update).to_hex(), accent, "{seed}");

        let [red, green, blue, _] = c(Role::Error).to_rgba8().map(u32::from);
        assert!(red > green * 2 && red > blue * 2, "{seed}");
        assert!(contrast(c(Role::Error), white) >= 2.5, "{seed}");
        let [red, green, blue, _] = c(Role::Warning).to_rgba8();
        assert!(red > blue && green > blue, "{seed}");
    }
}
