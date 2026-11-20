use crate::model::{color_hex, Doc, SceneStroke};
use std::fmt::Write;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

fn is_dot(s: &SceneStroke) -> bool {
    s.points.iter().all(|p| *p == s.points[0])
}

fn f(v: i32) -> f64 {
    v as f64 / 16.0
}

pub fn svg(doc: &Doc) -> String {
    let sc = doc.scene();
    let mut o = String::new();
    let _ = write!(
        o,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n<rect width=\"{w}\" height=\"{h}\" fill=\"{bg}\"/>\n",
        w = sc.width,
        h = sc.height,
        bg = color_hex(doc.background)
    );
    for s in &sc.strokes {
        if s.points.is_empty() {
            continue;
        }
        let c = color_hex(s.color);
        let w = s.width as f64 / 16.0;
        let alpha = |attr: &str| {
            if s.opacity < 255 {
                format!(" {attr}=\"{}\"", s.opacity as f64 / 255.0)
            } else {
                String::new()
            }
        };
        if is_dot(s) {
            let (x, y) = s.points[0];
            let _ = writeln!(o, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{c}\"{}/>", f(x), f(y), w / 2.0, alpha("fill-opacity"));
        } else {
            let mut d = String::new();
            for (i, &(x, y)) in s.points.iter().enumerate() {
                let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { " L" }, f(x), f(y));
            }
            let _ = writeln!(
                o,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"{w}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{}/>",
                alpha("stroke-opacity")
            );
        }
    }
    o.push_str("</svg>\n");
    o
}

pub fn raster(doc: &Doc, scale: f32) -> Result<Pixmap, String> {
    let sc = doc.scene();
    let scale = if scale.is_finite() { scale.clamp(0.01, 16.0) } else { 1.0 };
    let pw = ((sc.width as f32 * scale).round() as u32).clamp(1, 32768);
    let ph = ((sc.height as f32 * scale).round() as u32).clamp(1, 32768);
    if pw as u64 * ph as u64 > 400_000_000 {
        return Err("image too large".into());
    }
    let mut pm = Pixmap::new(pw, ph).ok_or("cannot allocate image")?;
    let [r, g, b] = doc.background;
    pm.fill(Color::from_rgba8(r, g, b, 255));
    let tf = Transform::from_scale(scale, scale);
    for s in &sc.strokes {
        if s.points.is_empty() {
            continue;
        }
        let mut paint = Paint::default();
        paint.set_color_rgba8(s.color[0], s.color[1], s.color[2], s.opacity);
        paint.anti_alias = true;
        let w = s.width as f32 / 16.0;
        let p = |v: i32| v as f32 / 16.0;
        if is_dot(s) {
            let (x, y) = s.points[0];
            if let Some(path) = PathBuilder::from_circle(p(x), p(y), w / 2.0) {
                pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
            }
        } else {
            let mut pb = PathBuilder::new();
            for (i, &(x, y)) in s.points.iter().enumerate() {
                if i == 0 {
                    pb.move_to(p(x), p(y));
                } else {
                    pb.line_to(p(x), p(y));
                }
            }
            if let Some(path) = pb.finish() {
                let st = Stroke { width: w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
                pm.stroke_path(&path, &paint, &st, tf, None);
            }
        }
    }
    Ok(pm)
}

pub fn png(doc: &Doc, scale: f32) -> Result<Vec<u8>, String> {
    raster(doc, scale)?.encode_png().map_err(|e| e.to_string())
}

pub fn jpg(doc: &Doc, scale: f32, quality: u8) -> Result<Vec<u8>, String> {
    let pm = raster(doc, scale)?;
    if pm.width() > 65535 || pm.height() > 65535 {
        return Err("image too large for jpg".into());
    }
    let rgb: Vec<u8> = pm.data().chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
    let mut out = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
    enc.encode(&rgb, pm.width() as u16, pm.height() as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}
