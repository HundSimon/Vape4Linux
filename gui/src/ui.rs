//! Custom-painted widgets. Everything here is drawn manually so the interface
//! keeps one consistent, restrained look instead of inheriting a desktop theme.

use egui::epaint::RectShape;
use egui::{
    Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2,
};

use crate::theme::{bold, mix, strong_font, with_alpha, Pal};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Close,
    Minimize,
    Refresh,
    Check,
    Bolt,
}

/// A one pixel separator.
pub fn hairline(ui: &mut Ui, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, color);
}

/// Card container: raised surface, hairline border, generous padding.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(Pal::CARD)
        .stroke(Stroke::new(1.0, Pal::LINE))
        .corner_radius(CornerRadius::same(Pal::RADIUS_CARD))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, add)
        .inner
}

/// A soft blurred halo behind an element, used sparingly for emphasis.
pub fn glow(painter: &egui::Painter, rect: Rect, radius: u8, color: Color32, blur: f32) {
    painter.add(RectShape::filled(rect, CornerRadius::same(radius), color).with_blur_width(blur));
}

pub fn rounded_rect(
    painter: &egui::Painter,
    rect: Rect,
    radius: u8,
    fill: Color32,
    stroke: Option<Color32>,
) {
    painter.rect_filled(rect, CornerRadius::same(radius), fill);
    if let Some(color) = stroke {
        painter.rect_stroke(
            rect,
            CornerRadius::same(radius),
            Stroke::new(1.0, color),
            StrokeKind::Inside,
        );
    }
}

/// Status light with an optional slow breathing halo.
pub fn status_dot(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    color: Color32,
    breathe: bool,
    time: f64,
) {
    if breathe {
        let phase = ((time * 2.0).sin() as f32 + 1.0) * 0.5;
        let halo = radius + 3.0 + phase * 2.5;
        painter.circle_filled(
            center,
            halo,
            with_alpha(color, (46.0 * (1.0 - phase * 0.55)) as u8),
        );
    }
    painter.circle_filled(center, radius, color);
}

/// Indeterminate progress arc.
pub fn spinner(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32, time: f64) {
    const SEGMENTS: usize = 20;
    let start = (time * 3.4) as f32;
    let sweep = std::f32::consts::TAU * 0.72;
    let mut previous = Pos2::ZERO;
    for index in 0..=SEGMENTS {
        let t = index as f32 / SEGMENTS as f32;
        let angle = start + sweep * t;
        let point = center + Vec2::new(angle.cos(), angle.sin()) * radius;
        if index > 0 {
            let alpha = (24.0 + 231.0 * t) as u8;
            painter.line_segment(
                [previous, point],
                Stroke::new(1.8, with_alpha(color, alpha)),
            );
        }
        previous = point;
    }
}

/// Compact determinate progress bar used for multi-stage background work.
pub fn progress_bar(ui: &mut Ui, progress: f32, color: Color32) -> Response {
    let height = 7.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let progress = progress.clamp(0.0, 1.0);
    let painter = ui.painter();

    rounded_rect(
        painter,
        rect,
        Pal::RADIUS_SMALL,
        Pal::CARD_PRESS,
        Some(Pal::LINE),
    );
    if progress > 0.0 {
        let fill = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + rect.width() * progress, rect.bottom()),
        );
        rounded_rect(painter, fill, Pal::RADIUS_SMALL, color, None);
    }
    response
}

pub fn icon(painter: &egui::Painter, kind: Icon, center: Pos2, size: f32, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let s = size * 0.5;
    let p = |x: f32, y: f32| center + Vec2::new(x * s, y * s);
    match kind {
        Icon::Close => {
            painter.line_segment([p(-0.55, -0.55), p(0.55, 0.55)], stroke);
            painter.line_segment([p(-0.55, 0.55), p(0.55, -0.55)], stroke);
        }
        Icon::Minimize => {
            painter.line_segment([p(-0.6, 0.0), p(0.6, 0.0)], stroke);
        }
        Icon::Refresh => {
            let mut points = Vec::with_capacity(22);
            let start = 0.66_f32;
            let sweep = std::f32::consts::TAU * 0.74;
            for index in 0..=21 {
                let angle = start + sweep * (index as f32 / 21.0);
                points.push(center + Vec2::new(angle.cos(), angle.sin()) * (s * 0.82));
            }
            painter.add(Shape::line(points, stroke));
            let tip = center + Vec2::new(start.cos(), start.sin()) * (s * 0.82);
            painter.add(Shape::convex_polygon(
                vec![
                    tip + Vec2::new(-s * 0.32, -s * 0.06),
                    tip + Vec2::new(s * 0.30, -s * 0.14),
                    tip + Vec2::new(s * 0.02, s * 0.36),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Check => {
            painter.line_segment([p(-0.6, 0.05), p(-0.15, 0.5)], stroke);
            painter.line_segment([p(-0.15, 0.5), p(0.65, -0.45)], stroke);
        }
        Icon::Bolt => {
            painter.add(Shape::convex_polygon(
                vec![
                    p(-0.05, -0.85),
                    p(-0.55, 0.08),
                    p(-0.08, 0.08),
                    p(0.05, 0.85),
                    p(0.55, -0.10),
                    p(0.08, -0.10),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
}

fn hover_t(id: egui::Id, ctx: &egui::Context, hovered: bool) -> f32 {
    ctx.animate_bool_with_time(id, hovered, 0.12)
}

/// Full width accent button.
pub fn primary_button(ui: &mut Ui, label: &str, enabled: bool, busy: bool, time: f64) -> Response {
    let height = 40.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let hovered = enabled && response.hovered();
    let t = hover_t(response.id, ui.ctx(), hovered);
    let pressed = enabled && response.is_pointer_button_down_on();

    let painter = ui.painter();
    if enabled {
        let base = mix(Pal::ACCENT, Pal::ACCENT_HOVER, t);
        let fill = if pressed {
            mix(base, Pal::ACCENT_PRESS, 0.8)
        } else {
            base
        };
        rounded_rect(painter, rect, Pal::RADIUS_CONTROL, fill, None);
    } else {
        rounded_rect(
            painter,
            rect,
            Pal::RADIUS_CONTROL,
            Pal::CARD,
            Some(Pal::LINE),
        );
    }

    let text_color = if enabled {
        Color32::WHITE
    } else {
        Pal::TEXT_MUTE
    };
    if busy {
        let spinner_center = Pos2::new(
            rect.center().x - 8.0 - measure(ui, label) * 0.5,
            rect.center().y,
        );
        spinner(painter, spinner_center, 7.0, text_color, time);
        painter.text(
            Pos2::new(spinner_center.x + 20.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::new(13.5, bold()),
            text_color,
        );
    } else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::new(13.5, bold()),
            text_color,
        );
    }
    response
}

/// Measures a label with the button font so the busy state can stay centred.
fn measure(ui: &Ui, label: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(label.to_string(), FontId::new(13.5, bold()), Color32::WHITE)
        .size()
        .x
}

/// Small secondary button.
pub fn ghost_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    let text_width = measure(ui, label);
    let size = Vec2::new(text_width + 24.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = enabled && response.hovered();
    let t = hover_t(response.id, ui.ctx(), hovered);
    let painter = ui.painter();

    let fill = mix(Color32::TRANSPARENT, Pal::CARD_HOVER, t);
    let stroke = mix(Pal::LINE_SOFT, Pal::LINE, t);
    rounded_rect(painter, rect, Pal::RADIUS_SMALL, fill, Some(stroke));
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(12.0, strong_font().family),
        if enabled {
            Pal::TEXT_DIM
        } else {
            Pal::TEXT_MUTE
        },
    );
    response
}

/// Square, icon-only button used in the title bar and section headers.
pub fn icon_button(ui: &mut Ui, kind: Icon, size: f32, danger: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hovered = response.hovered();
    let t = hover_t(response.id, ui.ctx(), hovered);
    let painter = ui.painter();

    let base = if danger { Pal::RED } else { Pal::CARD_HOVER };
    if t > 0.01 {
        rounded_rect(
            painter,
            rect,
            Pal::RADIUS_SMALL,
            with_alpha(base, (t * 235.0) as u8),
            None,
        );
    }
    let tint = if hovered && danger {
        Color32::WHITE
    } else {
        mix(
            if danger {
                Pal::TEXT_DIM
            } else {
                Pal::TEXT_MUTE
            },
            Pal::TEXT,
            t,
        )
    };
    icon(painter, kind, rect.center(), size * 0.56, tint);
    response
}

/// Animated switch.
pub fn toggle(ui: &mut Ui, id_source: &str, value: &mut bool) -> Response {
    let width = 34.0;
    let height = 20.0;
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let t = ui
        .ctx()
        .animate_bool_with_time(egui::Id::new(id_source), *value, 0.14);
    let painter = ui.painter();

    let track = mix(Pal::CARD_PRESS, Pal::ACCENT, t);
    let stroke = mix(Pal::LINE, Pal::ACCENT, t);
    rounded_rect(painter, rect, (height * 0.5) as u8, track, Some(stroke));

    let knob_radius = height * 0.5 - 2.5;
    let min_x = rect.left() + height * 0.5;
    let max_x = rect.right() - height * 0.5;
    let center = Pos2::new(min_x + (max_x - min_x) * t, rect.center().y);
    painter.circle_filled(center, knob_radius, mix(Pal::TEXT_MUTE, Color32::WHITE, t));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    #[test]
    fn toggle_reports_a_changed_response_when_clicked() {
        let context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        let mut value = false;
        let mut toggle_rect = Rect::NOTHING;
        let mut output = context.run_ui(
            RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| {
                toggle_rect = toggle(ui, "test-toggle", &mut value).rect;
            },
        );
        output.textures_delta.clear();

        let click = toggle_rect.center();
        let input = RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                Event::PointerMoved(click),
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        };
        let mut changed = false;

        let mut output = context.run_ui(input, |ui| {
            changed = toggle(ui, "test-toggle", &mut value).changed();
        });
        output.textures_delta.clear();

        assert!(value);
        assert!(changed);
    }
}
