//! Design tokens, typography and the global egui style.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, Style,
    TextStyle, Visuals,
};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Neutral surfaces, one muted accent, and semantic status hues.
pub struct Pal;

impl Pal {
    pub const BG: Color32 = rgb(0x19, 0x19, 0x19);
    pub const SIDEBAR: Color32 = rgb(0x14, 0x14, 0x14);
    pub const CARD: Color32 = rgb(0x1D, 0x1D, 0x1D);
    pub const CARD_HOVER: Color32 = rgb(0x22, 0x22, 0x22);
    pub const CARD_PRESS: Color32 = rgb(0x27, 0x27, 0x27);
    pub const LINE: Color32 = rgb(0x2B, 0x2B, 0x2B);
    pub const LINE_SOFT: Color32 = rgb(0x21, 0x21, 0x21);

    pub const TEXT: Color32 = rgb(0xEC, 0xEC, 0xEC);
    pub const TEXT_DIM: Color32 = rgb(0xA1, 0xA1, 0xA1);
    pub const TEXT_MUTE: Color32 = rgb(0x69, 0x69, 0x69);

    pub const ACCENT: Color32 = rgb(0x05, 0x86, 0x69);
    pub const ACCENT_HOVER: Color32 = rgb(0x07, 0x98, 0x79);
    pub const ACCENT_PRESS: Color32 = rgb(0x04, 0x70, 0x59);

    pub const GREEN: Color32 = rgb(0x40, 0xD8, 0x8C);
    pub const AMBER: Color32 = rgb(0xF5, 0xB5, 0x44);
    pub const RED: Color32 = rgb(0xF2, 0x6A, 0x5E);

    pub const RADIUS_CARD: u8 = 4;
    pub const RADIUS_CONTROL: u8 = 3;
    pub const RADIUS_SMALL: u8 = 3;
}

/// Linear blend between two colours, used for hover/press transitions.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

/// Applies a translucency factor without changing the hue.
pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn medium() -> FontFamily {
    FontFamily::Name("openvape-medium".into())
}

pub fn bold() -> FontFamily {
    FontFamily::Name("openvape-bold".into())
}

pub fn title_font() -> FontId {
    FontId::new(15.0, medium())
}

pub fn strong_font() -> FontId {
    FontId::new(13.0, medium())
}

pub fn mono_font() -> FontId {
    FontId::new(11.0, FontFamily::Monospace)
}

pub fn label_font() -> FontId {
    FontId::new(10.0, medium())
}

/// Installs fonts and the dark visual style.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    install_style(ctx);
}

fn install_style(ctx: &egui::Context) {
    let mut style = Style::default();
    let mut visuals = Visuals::dark();

    visuals.panel_fill = Pal::BG;
    visuals.window_fill = Pal::CARD;
    visuals.extreme_bg_color = rgb(0x0D, 0x0D, 0x11);
    visuals.faint_bg_color = Pal::LINE_SOFT;
    visuals.override_text_color = Some(Pal::TEXT);
    visuals.selection.bg_fill = with_alpha(Pal::ACCENT, 70);
    visuals.selection.stroke = Stroke::new(1.0, Pal::TEXT);
    visuals.window_stroke = Stroke::new(1.0, Pal::LINE);
    visuals.window_corner_radius = CornerRadius::same(Pal::RADIUS_CARD);
    visuals.menu_corner_radius = CornerRadius::same(Pal::RADIUS_CONTROL);
    visuals.hyperlink_color = Pal::ACCENT_HOVER;
    visuals.slider_trailing_fill = true;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = Pal::CARD;
    widgets.noninteractive.weak_bg_fill = Pal::CARD;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, Pal::LINE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, Pal::TEXT_DIM);
    widgets.noninteractive.corner_radius = CornerRadius::same(Pal::RADIUS_CONTROL);

    widgets.inactive.bg_fill = Pal::CARD;
    widgets.inactive.weak_bg_fill = Pal::CARD;
    widgets.inactive.bg_stroke = Stroke::new(1.0, Pal::LINE);
    widgets.inactive.fg_stroke = Stroke::new(1.0, Pal::TEXT);
    widgets.inactive.corner_radius = CornerRadius::same(Pal::RADIUS_CONTROL);

    widgets.hovered.bg_fill = Pal::CARD_HOVER;
    widgets.hovered.weak_bg_fill = Pal::CARD_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.0, rgb(0x32, 0x32, 0x3E));
    widgets.hovered.fg_stroke = Stroke::new(1.0, Pal::TEXT);
    widgets.hovered.corner_radius = CornerRadius::same(Pal::RADIUS_CONTROL);

    widgets.active.bg_fill = Pal::CARD_PRESS;
    widgets.active.weak_bg_fill = Pal::CARD_PRESS;
    widgets.active.bg_stroke = Stroke::new(1.0, rgb(0x3C, 0x3C, 0x4A));
    widgets.active.fg_stroke = Stroke::new(1.0, Pal::TEXT);
    widgets.active.corner_radius = CornerRadius::same(Pal::RADIUS_CONTROL);

    widgets.open.bg_fill = Pal::CARD_HOVER;
    widgets.open.weak_bg_fill = Pal::CARD_HOVER;
    widgets.open.bg_stroke = Stroke::new(1.0, Pal::LINE);

    style.visuals = visuals;

    let mut text_styles = std::collections::BTreeMap::new();
    text_styles.insert(
        TextStyle::Small,
        FontId::new(11.0, FontFamily::Proportional),
    );
    text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    text_styles.insert(TextStyle::Button, FontId::new(13.0, medium()));
    text_styles.insert(TextStyle::Heading, FontId::new(16.0, medium()));
    text_styles.insert(TextStyle::Monospace, mono_font());
    text_styles.insert(TextStyle::Name("label".into()), FontId::new(10.0, medium()));
    text_styles.insert(TextStyle::Name("title".into()), title_font());
    text_styles.insert(TextStyle::Name("strong".into()), strong_font());
    style.text_styles = text_styles;

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.window_margin = Margin::same(0);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.interact_size = egui::vec2(28.0, 26.0);
    style.spacing.scroll.bar_width = 5.0;
    style.spacing.scroll.floating = true;
    style.spacing.scroll.bar_inner_margin = 3.0;
    style.animation_time = 0.13;

    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let regular = resolve_font(&[
        "NotoSans-Regular",
        "Inter-Regular",
        "InterVariable",
        "DejaVuSans",
        "LiberationSans-Regular",
    ]);
    let semibold = resolve_font(&[
        "NotoSans-SemiBold",
        "Inter-SemiBold",
        "NotoSans-Medium",
        "Inter-Medium",
        "DejaVuSans-Bold",
        "LiberationSans-Bold",
    ]);
    let mono = resolve_font(&[
        "NotoSansMono-Regular",
        "JetBrainsMono-Regular",
        "FiraMono-Regular",
        "DejaVuSansMono",
        "LiberationMono-Regular",
    ]);

    let mut families = fonts.families.clone();

    if let Some(bytes) = regular {
        let name = "openvape-sans".to_string();
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = families.get_mut(&FontFamily::Proportional) {
            list.insert(0, name.clone());
        }
        if let Some(list) = families.get_mut(&FontFamily::Monospace) {
            list.push(name.clone());
        }
        families.insert(
            FontFamily::Name("openvape-fallback".into()),
            vec![name.clone()],
        );
        install_named_family(&mut fonts, &mut families, "openvape-regular", &name);
    }

    // Named families need at least one entry, so fall back to the proportional
    // stack when a dedicated semibold face is unavailable.
    let fallback_stack = families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_else(|| vec!["Ubuntu-Light".to_string()]);

    let medium_name = if let Some(bytes) = semibold {
        let name = "openvape-sans-semibold".to_string();
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        name
    } else {
        fallback_stack
            .first()
            .cloned()
            .unwrap_or_else(|| "Ubuntu-Light".to_string())
    };

    let mut medium_stack = vec![medium_name];
    medium_stack.extend(fallback_stack.iter().cloned());
    families.insert(
        FontFamily::Name("openvape-medium".into()),
        medium_stack.clone(),
    );
    families.insert(FontFamily::Name("openvape-bold".into()), medium_stack);

    if let Some(bytes) = mono {
        let name = "openvape-mono".to_string();
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = families.get_mut(&FontFamily::Monospace) {
            list.insert(0, name);
        }
    }

    fonts.families = families;
    ctx.set_fonts(fonts);
}

fn install_named_family(
    _fonts: &mut FontDefinitions,
    families: &mut std::collections::BTreeMap<FontFamily, Vec<String>>,
    family: &str,
    font: &str,
) {
    families.insert(FontFamily::Name(family.into()), vec![font.to_string()]);
}

fn resolve_font(stems: &[&str]) -> Option<Vec<u8>> {
    for stem in stems {
        if let Some(path) = search_font(stem) {
            if let Ok(bytes) = std::fs::read(&path) {
                return Some(bytes);
            }
        }
    }
    None
}

fn search_font(stem: &str) -> Option<PathBuf> {
    let lowered = stem.to_lowercase();
    let roots = [
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("/usr/share/fonts/TTF"),
        PathBuf::from("/usr/share/fonts/OTF"),
        crate::util::home_dir().join(".local/share/fonts"),
        crate::util::home_dir().join(".fonts"),
    ];
    for root in roots {
        if let Some(found) = walk_for_font(&root, &lowered, 0) {
            return Some(found);
        }
    }
    None
}

fn walk_for_font(dir: &Path, stem: &str, depth: usize) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            files.push(path);
        }
    }
    // A shallow file always wins over descending further.
    for path in &files {
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if (extension == "ttf" || extension == "otf") && name == stem {
            return Some(path.clone());
        }
    }
    for path in &files {
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if (extension == "ttf" || extension == "otf") && name.starts_with(stem) {
            return Some(path.clone());
        }
    }
    for dir in dirs {
        if let Some(found) = walk_for_font(&dir, stem, depth + 1) {
            return Some(found);
        }
    }
    None
}
