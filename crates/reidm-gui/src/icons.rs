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


/// The per-item progress bar drawn under a file name.
pub fn progress_bar(p: &Painter, rect: Rect, fraction: f32, fg: Color32, bg: Color32) {
    p.rect_filled(rect, 1.0, bg);
    let w = rect.width() * fraction.clamp(0.0, 1.0);
    if w > 0.0 {
        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, rect.height())), 1.0, fg);
    }
}
