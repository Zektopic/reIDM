//! Toolbar and sidebar glyphs drawn with the painter, so the app ships without image assets and
//! stays crisp at any DPI.
//!
//! Every function paints into a caller-supplied [`Rect`] and never allocates space, so an icon can
//! never end up somewhere other than where its button reserved room for it.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// A plus sign — "add".
pub fn plus(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let d = r.width() * 0.28;
    let s = Stroke::new(r.width() * 0.13, color);
    p.line_segment([Pos2::new(c.x - d, c.y), Pos2::new(c.x + d, c.y)], s);
    p.line_segment([Pos2::new(c.x, c.y - d), Pos2::new(c.x, c.y + d)], s);
}

/// Downward triangle — "start / resume".
pub fn play(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let d = r.width() * 0.30;
    p.add(Shape::convex_polygon(
        vec![Pos2::new(c.x - d, c.y - d), Pos2::new(c.x + d, c.y - d), Pos2::new(c.x, c.y + d)],
        color,
        Stroke::NONE,
    ));
}

/// Two vertical bars — "stop / pause".
pub fn pause(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let (w, h) = (r.width() * 0.14, r.height() * 0.60);
    for dx in [-r.width() * 0.13, r.width() * 0.13] {
        p.rect_filled(Rect::from_center_size(Pos2::new(c.x + dx, c.y), Vec2::new(w, h)), 1.0, color);
    }
}

/// An X — "remove".
pub fn cross(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let d = r.width() * 0.28;
    let s = Stroke::new(r.width() * 0.13, color);
    p.line_segment([Pos2::new(c.x - d, c.y - d), Pos2::new(c.x + d, c.y + d)], s);
    p.line_segment([Pos2::new(c.x - d, c.y + d), Pos2::new(c.x + d, c.y - d)], s);
}

/// A magnifier — "search".
pub fn search(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let rad = r.width() * 0.22;
    let s = Stroke::new(r.width() * 0.11, color);
    p.circle_stroke(c, rad, s);
    let d = rad * 0.72;
    p.line_segment([Pos2::new(c.x + d, c.y + d), Pos2::new(c.x + d + rad * 0.6, c.y + d + rad * 0.6)], s);
}

/// A folder outline — "open / show in folder".
pub fn folder(p: &Painter, r: Rect, color: Color32) {
    let b = r.shrink(r.width() * 0.20);
    let s = Stroke::new(r.width() * 0.10, color);
    p.line_segment(
        [Pos2::new(b.left(), b.top() + b.height() * 0.22), Pos2::new(b.left() + b.width() * 0.45, b.top() + b.height() * 0.22)],
        s,
    );
    p.line_segment([Pos2::new(b.left() + b.width() * 0.45, b.top()), Pos2::new(b.right(), b.top())], s);
    p.rect_stroke(b, 1.0, s, egui::StrokeKind::Middle);
}

/// A gear-ish "options" glyph: a ring with spokes.
pub fn gear(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let s = Stroke::new(r.width() * 0.10, color);
    p.circle_stroke(c, r.width() * 0.20, s);
    let (outer, inner) = (r.width() * 0.34, r.width() * 0.24);
    for k in 0..8 {
        let a = std::f32::consts::TAU * k as f32 / 8.0;
        let (sn, cs) = a.sin_cos();
        p.line_segment(
            [Pos2::new(c.x + cs * inner, c.y + sn * inner), Pos2::new(c.x + cs * outer, c.y + sn * outer)],
            s,
        );
    }
}

/// A filled circle used as a status dot in the list.
pub fn dot(p: &Painter, r: Rect, color: Color32) {
    p.circle_filled(r.center(), r.width() * 0.30, color);
}


/// Two pause bars with a bracket: "stop everything".
pub fn pause_all(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let (w, h) = (r.width() * 0.11, r.height() * 0.52);
    for dx in [-r.width() * 0.20, r.width() * 0.02] {
        p.rect_filled(Rect::from_center_size(Pos2::new(c.x + dx, c.y), Vec2::new(w, h)), 1.0, color);
    }
    let s = Stroke::new(r.width() * 0.11, color);
    let x = c.x + r.width() * 0.26;
    p.line_segment([Pos2::new(x, c.y - h * 0.5), Pos2::new(x, c.y + h * 0.5)], s);
    for dy in [-h * 0.34, h * 0.34] {
        p.line_segment(
            [Pos2::new(c.x - r.width() * 0.30, c.y + dy), Pos2::new(x, c.y + dy)],
            s,
        );
    }
}

/// An X with a bracket: "delete every completed entry".
pub fn cross_all(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let s = Stroke::new(r.width() * 0.12, color);
    let d = r.width() * 0.20;
    let x = c.x + r.width() * 0.02;
    p.line_segment([Pos2::new(x - d, c.y - d), Pos2::new(x + d, c.y + d)], s);
    p.line_segment([Pos2::new(x - d, c.y + d), Pos2::new(x + d, c.y - d)], s);
    let bx = c.x + r.width() * 0.30;
    for dy in [-d, d] {
        p.line_segment([Pos2::new(x - r.width() * 0.34, c.y + dy), Pos2::new(bx, c.y + dy)], s);
    }
    p.line_segment([Pos2::new(bx, c.y - d), Pos2::new(bx, c.y + d)], s);
}

/// A clock face — "scheduler".
pub fn clock(p: &Painter, r: Rect, color: Color32) {
    let c = r.center();
    let s = Stroke::new(r.width() * 0.11, color);
    p.circle_stroke(c, r.width() * 0.34, s);
    p.line_segment([c, Pos2::new(c.x, c.y - r.width() * 0.20)], s);
    p.line_segment([c, Pos2::new(c.x + r.width() * 0.16, c.y + r.width() * 0.06)], s);
}

/// List lines with a play triangle: "start the scheduled queue".
pub fn queue_start(p: &Painter, r: Rect, color: Color32) {
    queue_lines(p, r, color);
    let (cx, cy, d) = (r.left() + r.width() * 0.70, r.center().y, r.width() * 0.17);
    p.add(Shape::convex_polygon(
        vec![Pos2::new(cx - d * 0.7, cy - d), Pos2::new(cx + d, cy), Pos2::new(cx - d * 0.7, cy + d)],
        color,
        Stroke::NONE,
    ));
}

/// List lines with two bars: "stop the scheduled queue".
pub fn queue_stop(p: &Painter, r: Rect, color: Color32) {
    queue_lines(p, r, color);
    let (w, h) = (r.width() * 0.09, r.height() * 0.40);
    for dx in [0.62, 0.80] {
        p.rect_filled(Rect::from_center_size(Pos2::new(r.left() + r.width() * dx, r.center().y), Vec2::new(w, h)), 1.0, color);
    }
}

fn queue_lines(p: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(r.width() * 0.10, color);
    let (left, right) = (r.left() + r.width() * 0.12, r.left() + r.width() * 0.46);
    for k in 0..3 {
        let y = r.top() + r.height() * (0.30 + 0.20 * k as f32);
        p.line_segment([Pos2::new(left, y), Pos2::new(right, y)], s);
    }
}

/// A gauge with its needle low: the speed limiter.
pub fn gauge(p: &Painter, r: Rect, color: Color32) {
    let c = Pos2::new(r.center().x, r.top() + r.height() * 0.66);
    let rad = r.width() * 0.36;
    let s = Stroke::new(r.width() * 0.10, color);
    let pts: Vec<Pos2> = (0..=16)
        .map(|k| {
            let a = std::f32::consts::PI * (1.0 + k as f32 / 16.0);
            Pos2::new(c.x + rad * a.cos(), c.y + rad * a.sin())
        })
        .collect();
    p.add(Shape::line(pts, s));
    let a = std::f32::consts::PI * 1.25;
    p.line_segment([c, Pos2::new(c.x + rad * 0.8 * a.cos(), c.y + rad * 0.8 * a.sin())], s);
    p.circle_filled(c, r.width() * 0.07, color);
}

/// A magnifier over a grid: the site grabber's "find everything on this page" idea.
pub fn grabber(p: &Painter, r: Rect, color: Color32) {
    let s = Stroke::new(r.width() * 0.10, color);
    let b = r.shrink(r.width() * 0.22);
    let cell = b.width() / 2.0;
    for (i, j) in [(0, 0), (1, 0), (0, 1)] {
        p.rect_stroke(
            Rect::from_min_size(Pos2::new(b.left() + i as f32 * cell, b.top() + j as f32 * cell), Vec2::splat(cell * 0.82)),
            1.0,
            s,
            egui::StrokeKind::Middle,
        );
    }
    let c = Pos2::new(b.right() - cell * 0.15, b.bottom() - cell * 0.15);
    p.circle_stroke(c, r.width() * 0.16, s);
    p.line_segment([Pos2::new(c.x + r.width() * 0.11, c.y + r.width() * 0.11), Pos2::new(c.x + r.width() * 0.24, c.y + r.width() * 0.24)], s);
}

/// A small "file card" tinted by category, the way IDM marks each row with a file-type
/// icon. A filled rounded card with a white mark stays readable down to 16 px, where a
/// literal page outline would turn to mush.
pub fn file_card(p: &Painter, r: Rect, cat: crate::model::Category, dark: bool) {
    use crate::model::Category as C;
    let fill = match cat {
        C::Video => Color32::from_rgb(0x8B, 0x5C, 0xF6),
        C::Music => Color32::from_rgb(0xE0, 0x4A, 0x9B),
        C::Documents => Color32::from_rgb(0x2A, 0x84, 0xE0),
        C::Compressed => Color32::from_rgb(0xD9, 0x8A, 0x1F),
        C::Programs => Color32::from_rgb(0x2E, 0xA0, 0x53),
        C::Other | C::All => {
            if dark {
                Color32::from_rgb(0x6E, 0x76, 0x82)
            } else {
                Color32::from_rgb(0x8A, 0x93, 0xA0)
            }
        }
    };
    let w = r.width();
    let card = Rect::from_center_size(r.center(), Vec2::new(w * 0.72, w * 0.88));
    p.rect_filled(card, w * 0.14, fill);

    let ink = Color32::WHITE;
    let c = card.center();
    match cat {
        // A play triangle.
        C::Video => {
            p.add(Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - w * 0.09, c.y - w * 0.13),
                    Pos2::new(c.x - w * 0.09, c.y + w * 0.13),
                    Pos2::new(c.x + w * 0.13, c.y),
                ],
                ink,
                Stroke::NONE,
            ));
        }
        // A note: stem plus head.
        C::Music => {
            p.line_segment(
                [Pos2::new(c.x + w * 0.08, c.y - w * 0.15), Pos2::new(c.x + w * 0.08, c.y + w * 0.09)],
                Stroke::new(w * 0.07, ink),
            );
            p.line_segment(
                [Pos2::new(c.x + w * 0.08, c.y - w * 0.15), Pos2::new(c.x - w * 0.10, c.y - w * 0.09)],
                Stroke::new(w * 0.07, ink),
            );
            p.circle_filled(Pos2::new(c.x + w * 0.01, c.y + w * 0.11), w * 0.08, ink);
        }
        // Text lines.
        C::Documents => {
            let s = Stroke::new(w * 0.07, ink);
            for (i, dy) in [-0.12f32, 0.0, 0.12].iter().enumerate() {
                let half = if i == 2 { w * 0.10 } else { w * 0.16 };
                p.line_segment([Pos2::new(c.x - half, c.y + w * dy), Pos2::new(c.x + half, c.y + w * dy)], s);
            }
        }
        // Two stacked bars.
        C::Compressed => {
            for dy in [-0.07f32, 0.07] {
                p.rect_filled(
                    Rect::from_center_size(Pos2::new(c.x, c.y + w * dy), Vec2::new(w * 0.34, w * 0.10)),
                    w * 0.03,
                    ink,
                );
            }
        }
        // A gear-ish square with a notch, distinct from the toolbar gear at this size.
        C::Programs => {
            p.rect_filled(Rect::from_center_size(c, Vec2::splat(w * 0.34)), w * 0.07, ink);
            p.rect_filled(Rect::from_center_size(c, Vec2::splat(w * 0.14)), w * 0.03, fill);
        }
        C::Other | C::All => {
            let s = Stroke::new(w * 0.07, ink);
            p.line_segment([Pos2::new(c.x - w * 0.14, c.y), Pos2::new(c.x + w * 0.14, c.y)], s);
        }
    }
}

/// A hollow box for the row-selection column; `check` is `Some` for on, `None` for the
/// header's "some but not all" state.
pub fn checkbox(p: &Painter, r: Rect, check: Option<bool>, color: Color32, border: Color32) {
    let box_ = Rect::from_center_size(r.center(), Vec2::splat(r.width() * 0.58));
    p.rect_filled(box_, 2.0, if check.is_some() { color.gamma_multiply(0.10) } else { Color32::TRANSPARENT });
    p.rect_stroke(box_, 2.0, Stroke::new(1.0, if check.is_some() { color } else { border }), egui::StrokeKind::Inside);
    if check == Some(true) {
        let s = Stroke::new(1.8, color);
        let c = box_.center();
        p.line_segment([Pos2::new(c.x - box_.width() * 0.22, c.y), Pos2::new(c.x - box_.width() * 0.04, c.y + box_.height() * 0.20)], s);
        p.line_segment([Pos2::new(c.x - box_.width() * 0.04, c.y + box_.height() * 0.20), Pos2::new(c.x + box_.width() * 0.24, c.y - box_.height() * 0.22)], s);
    } else if check.is_none() {
        let c = box_.center();
        p.rect_filled(
            Rect::from_center_size(c, Vec2::new(box_.width() * 0.45, box_.height() * 0.16)),
            1.0,
            color,
        );
    }
}

/// The per-item progress bar drawn under a file name.
pub fn progress_bar(p: &Painter, rect: Rect, fraction: f32, fg: Color32, bg: Color32) {
    p.rect_filled(rect, 1.0, bg);
    let w = rect.width() * fraction.clamp(0.0, 1.0);
    if w > 0.0 {
        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, rect.height())), 1.0, fg);
    }
}

/// A chevron pointing down (expanded) or right (collapsed), for tree expanders.
pub fn chevron(p: &Painter, r: Rect, color: Color32, down: bool) {
    let c = r.center();
    let d = r.width() * 0.26;
    let s = Stroke::new(r.width() * 0.14, color);
    if down {
        p.line_segment([Pos2::new(c.x - d, c.y - d * 0.5), Pos2::new(c.x, c.y + d * 0.5)], s);
        p.line_segment([Pos2::new(c.x + d, c.y - d * 0.5), Pos2::new(c.x, c.y + d * 0.5)], s);
    } else {
        p.line_segment([Pos2::new(c.x - d * 0.5, c.y - d), Pos2::new(c.x + d * 0.5, c.y)], s);
        p.line_segment([Pos2::new(c.x - d * 0.5, c.y + d), Pos2::new(c.x + d * 0.5, c.y)], s);
    }
}
