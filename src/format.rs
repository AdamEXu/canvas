use crate::model::{Action, Doc, Op};

const MAGIC: &[u8; 4] = b"CNVS";
const VERSION: u8 = 2;

fn put(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn zz(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzz(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let v = *self.b.get(self.i).ok_or("truncated")?;
        self.i += 1;
        Ok(v)
    }
    fn var(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("bad varint".into())
    }
    fn svar(&mut self) -> Result<i64, String> {
        Ok(unzz(self.var()?))
    }
}

pub fn encode(doc: &Doc) -> Vec<u8> {
    let mut p = Vec::new();
    put(&mut p, doc.width as u64);
    put(&mut p, doc.height as u64);
    p.extend_from_slice(&doc.background);
    put(&mut p, doc.actions.len() as u64);
    let mut last_t = 0i64;
    for a in &doc.actions {
        let ty = match &a.op {
            Op::Stroke { eraser: false, opacity: 255, .. } => 0,
            Op::Stroke { eraser: true, opacity: 255, .. } => 1,
            Op::Stroke { eraser: false, .. } => 6,
            Op::Stroke { eraser: true, .. } => 7,
            Op::Resize { .. } => 2,
            Op::Clear => 3,
            Op::Undo => 4,
            Op::Redo => 5,
        };
        p.push(ty);
        put(&mut p, zz(a.t as i64 - last_t));
        last_t = a.t as i64;
        match &a.op {
            Op::Stroke { color, width, points, opacity, .. } => {
                if *opacity < 255 {
                    p.push(*opacity);
                }
                p.extend_from_slice(color);
                put(&mut p, *width as u64);
                put(&mut p, points.len() as u64);
                let (mut px, mut py) = (0i64, 0i64);
                for &(x, y) in points {
                    put(&mut p, zz(x as i64 - px));
                    put(&mut p, zz(y as i64 - py));
                    px = x as i64;
                    py = y as i64;
                }
            }
            Op::Resize { left, top, right, bottom } => {
                for v in [left, top, right, bottom] {
                    put(&mut p, zz(*v as i64));
                }
            }
            _ => {}
        }
    }
    let mut out = MAGIC.to_vec();
    out.push(VERSION);
    out.extend(miniz_oxide::deflate::compress_to_vec(&p, 9));
    out
}

pub fn decode(bytes: &[u8]) -> Result<Doc, String> {
    if bytes.len() < 5 || &bytes[..4] != MAGIC {
        return Err("not a .canvas file".into());
    }
    let version = bytes[4];
    if !(1..=VERSION).contains(&version) {
        return Err(format!("unsupported version {}", bytes[4]));
    }
    let p = miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes[5..], 1 << 30)
        .map_err(|e| format!("decompress: {e:?}"))?;
    let mut r = Reader { b: &p, i: 0 };
    let width = r.var()? as u32;
    let height = r.var()? as u32;
    let background = [r.byte()?, r.byte()?, r.byte()?];
    let n = r.var()? as usize;
    let mut actions = Vec::with_capacity(n.min(1 << 20));
    let mut last_t = 0i64;
    for _ in 0..n {
        let ty = r.byte()?;
        last_t += r.svar()?;
        let op = match ty {
            0 | 1 | 6 | 7 if ty < 6 || version >= 2 => {
                let opacity = if ty >= 6 { r.byte()? } else { 255 };
                let color = [r.byte()?, r.byte()?, r.byte()?];
                let width = r.var()? as u32;
                let count = r.var()? as usize;
                let mut points = Vec::with_capacity(count.min(1 << 20));
                let (mut px, mut py) = (0i64, 0i64);
                for _ in 0..count {
                    px += r.svar()?;
                    py += r.svar()?;
                    points.push((px as i32, py as i32));
                }
                Op::Stroke { color, width, points, eraser: ty % 2 == 1, opacity }
            }
            2 => Op::Resize {
                left: r.svar()? as i32,
                top: r.svar()? as i32,
                right: r.svar()? as i32,
                bottom: r.svar()? as i32,
            },
            3 => Op::Clear,
            4 => Op::Undo,
            5 => Op::Redo,
            _ => return Err(format!("unknown action type {ty}")),
        };
        actions.push(Action { t: last_t as u64, op });
    }
    let mut d = Doc::new(width, height, background);
    d.actions = actions;
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut d = Doc::new(800, 600, [250, 240, 230]);
        d.actions = vec![
            Action {
                t: 1_700_000_000_000,
                op: Op::Stroke {
                    color: [1, 2, 3],
                    width: 65,
                    points: vec![(0, 0), (-17, 33), (160000, -5), (i32::MAX, i32::MIN)],
                    eraser: false,
                    opacity: 255,
                },
            },
            Action {
                t: 1_700_000_000_100,
                op: Op::Stroke { color: [9, 8, 7], width: 32, points: vec![(1, 1), (40, 2)], eraser: false, opacity: 128 },
            },
            Action { t: 1_700_000_000_500, op: Op::Resize { left: -10, top: 5, right: 0, bottom: 100 } },
            Action {
                t: 1_699_999_999_000,
                op: Op::Stroke { color: [255, 255, 255], width: 16, points: vec![(5, 5)], eraser: true, opacity: 0 },
            },
            Action { t: 1_700_000_001_000, op: Op::Clear },
            Action { t: 1_700_000_002_000, op: Op::Undo },
            Action { t: 1_700_000_003_000, op: Op::Redo },
        ];
        let back = decode(&encode(&d)).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn opaque_strokes_cost_no_extra_bytes() {
        let stroke = |opacity| Doc {
            actions: vec![Action {
                t: 5,
                op: Op::Stroke { color: [1, 2, 3], width: 16, points: vec![(0, 0), (16, 16)], eraser: false, opacity },
            }],
            ..Doc::new(10, 10, [255, 255, 255])
        };
        let raw = |d: &Doc| miniz_oxide::inflate::decompress_to_vec(&encode(d)[5..]).unwrap();
        assert_eq!(raw(&stroke(200)).len(), raw(&stroke(255)).len() + 1);
    }

    #[test]
    fn decodes_v1() {
        let mut p = Vec::new();
        put(&mut p, 100);
        put(&mut p, 50);
        p.extend_from_slice(&[255, 255, 255]);
        put(&mut p, 2);
        p.push(0);
        put(&mut p, zz(1000));
        p.extend_from_slice(&[10, 20, 30]);
        put(&mut p, 64);
        put(&mut p, 2);
        for v in [16, 32, 16, 0] {
            put(&mut p, zz(v));
        }
        p.push(1);
        put(&mut p, zz(5));
        p.extend_from_slice(&[0, 0, 0]);
        put(&mut p, 16);
        put(&mut p, 1);
        put(&mut p, zz(8));
        put(&mut p, zz(8));
        let mut bytes = MAGIC.to_vec();
        bytes.push(1);
        bytes.extend(miniz_oxide::deflate::compress_to_vec(&p, 9));
        let d = decode(&bytes).unwrap();
        assert_eq!((d.width, d.height), (100, 50));
        assert_eq!(
            d.actions,
            vec![
                Action {
                    t: 1000,
                    op: Op::Stroke { color: [10, 20, 30], width: 64, points: vec![(16, 32), (32, 32)], eraser: false, opacity: 255 },
                },
                Action { t: 1005, op: Op::Stroke { color: [0, 0, 0], width: 16, points: vec![(8, 8)], eraser: true, opacity: 255 } },
            ]
        );
    }
}
