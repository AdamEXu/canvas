use serde::Deserialize;
use serde_json::{json, Value};

pub const MAX_SIZE: i64 = 16384;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Stroke { color: [u8; 3], width: u32, points: Vec<(i32, i32)>, eraser: bool },
    Resize { left: i32, top: i32, right: i32, bottom: i32 },
    Clear,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub t: u64,
    pub op: Op,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub doc: u64,
    pub width: u32,
    pub height: u32,
    pub background: [u8; 3],
    pub actions: Vec<Action>,
}

pub struct SceneStroke {
    pub color: [u8; 3],
    pub width: u32,
    pub points: Vec<(i32, i32)>,
}

pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub strokes: Vec<SceneStroke>,
    pub can_undo: bool,
    pub can_redo: bool,
}

fn def_color() -> String {
    "#000000".into()
}
fn def_width() -> f64 {
    4.0
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum In {
    Stroke {
        #[serde(default = "def_color")]
        color: String,
        #[serde(default = "def_width")]
        width: f64,
        #[serde(default)]
        points: Vec<[f64; 2]>,
        #[serde(default)]
        eraser: bool,
    },
    Resize {
        #[serde(default)]
        left: f64,
        #[serde(default)]
        top: f64,
        #[serde(default)]
        right: f64,
        #[serde(default)]
        bottom: f64,
    },
    Clear {},
    Undo {},
    Redo {},
}

pub fn q(v: f64) -> i32 {
    (v * 16.0).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    let h: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn color_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn parse_op(v: Value) -> Result<Op, String> {
    let i: In = serde_json::from_value(v).map_err(|e| e.to_string())?;
    Ok(match i {
        In::Stroke { color, width, points, eraser } => Op::Stroke {
            color: parse_color(&color).ok_or("bad color")?,
            width: q(width).clamp(1, 16 * 4096) as u32,
            points: points.iter().map(|p| (q(p[0]), q(p[1]))).collect(),
            eraser,
        },
        In::Resize { left, top, right, bottom } => Op::Resize {
            left: left.round() as i32,
            top: top.round() as i32,
            right: right.round() as i32,
            bottom: bottom.round() as i32,
        },
        In::Clear {} => Op::Clear,
        In::Undo {} => Op::Undo,
        In::Redo {} => Op::Redo,
    })
}

pub fn action_json(a: &Action) -> Value {
    match &a.op {
        Op::Stroke { color, width, points, eraser } => json!({
            "type": "stroke",
            "color": color_hex(*color),
            "width": *width as f64 / 16.0,
            "points": points.iter().map(|p| [p.0 as f64 / 16.0, p.1 as f64 / 16.0]).collect::<Vec<_>>(),
            "eraser": eraser,
            "t": a.t,
        }),
        Op::Resize { left, top, right, bottom } => json!({
            "type": "resize", "left": left, "top": top, "right": right, "bottom": bottom, "t": a.t,
        }),
        Op::Clear => json!({"type": "clear", "t": a.t}),
        Op::Undo => json!({"type": "undo", "t": a.t}),
        Op::Redo => json!({"type": "redo", "t": a.t}),
    }
}

impl Doc {
    pub fn new(width: u32, height: u32, background: [u8; 3]) -> Doc {
        Doc {
            doc: 0,
            width: width.clamp(1, MAX_SIZE as u32),
            height: height.clamp(1, MAX_SIZE as u32),
            background,
            actions: Vec::new(),
        }
    }

    pub fn scene(&self) -> Scene {
        let mut eff: Vec<usize> = Vec::new();
        let mut redo: Vec<usize> = Vec::new();
        for (i, a) in self.actions.iter().enumerate() {
            match a.op {
                Op::Undo => {
                    if let Some(x) = eff.pop() {
                        redo.push(x)
                    }
                }
                Op::Redo => {
                    if let Some(x) = redo.pop() {
                        eff.push(x)
                    }
                }
                _ => {
                    redo.clear();
                    eff.push(i)
                }
            }
        }
        let mut w = self.width as i64;
        let mut h = self.height as i64;
        let mut strokes: Vec<SceneStroke> = Vec::new();
        for &i in &eff {
            match &self.actions[i].op {
                Op::Stroke { color, width, points, eraser } => strokes.push(SceneStroke {
                    color: if *eraser { self.background } else { *color },
                    width: *width,
                    points: points.clone(),
                }),
                Op::Resize { left, top, right, bottom } => {
                    w = (w + *left as i64 + *right as i64).clamp(1, MAX_SIZE);
                    h = (h + *top as i64 + *bottom as i64).clamp(1, MAX_SIZE);
                    let (dx, dy) = (left.saturating_mul(16), top.saturating_mul(16));
                    for s in &mut strokes {
                        for p in &mut s.points {
                            p.0 = p.0.saturating_add(dx);
                            p.1 = p.1.saturating_add(dy);
                        }
                    }
                }
                Op::Clear => strokes.clear(),
                _ => {}
            }
        }
        Scene {
            width: w as u32,
            height: h as u32,
            strokes,
            can_undo: !eff.is_empty(),
            can_redo: !redo.is_empty(),
        }
    }

    pub fn state_json(&self) -> Value {
        let s = self.scene();
        json!({
            "doc": self.doc,
            "version": self.actions.len(),
            "width": s.width,
            "height": s.height,
            "background": color_hex(self.background),
            "initial_width": self.width,
            "initial_height": self.height,
            "can_undo": s.can_undo,
            "can_redo": s.can_redo,
        })
    }
}
