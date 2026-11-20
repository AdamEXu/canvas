use crate::model::{Action, Doc, Op};

const MAGIC: &[u8; 4] = b"CNVS";
const VERSION: u8 = 1;

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
            Op::Stroke { eraser: false, .. } => 0,
            Op::Stroke { eraser: true, .. } => 1,
            Op::Resize { .. } => 2,
            Op::Clear => 3,
            Op::Undo => 4,
            Op::Redo => 5,
        };
        p.push(ty);
        put(&mut p, zz(a.t as i64 - last_t));
        last_t = a.t as i64;
        match &a.op {
            Op::Stroke { color, width, points, .. } => {
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
    if bytes[4] != VERSION {
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
            0 | 1 => {
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
                Op::Stroke { color, width, points, eraser: ty == 1 }
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
                },
            },
            Action { t: 1_700_000_000_500, op: Op::Resize { left: -10, top: 5, right: 0, bottom: 100 } },
            Action {
                t: 1_699_999_999_000,
                op: Op::Stroke { color: [255, 255, 255], width: 16, points: vec![(5, 5)], eraser: true },
            },
            Action { t: 1_700_000_001_000, op: Op::Clear },
            Action { t: 1_700_000_002_000, op: Op::Undo },
            Action { t: 1_700_000_003_000, op: Op::Redo },
        ];
        let back = decode(&encode(&d)).unwrap();
        assert_eq!(back, d);
    }
}
