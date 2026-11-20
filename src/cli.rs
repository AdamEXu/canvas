use crate::{format, model::Doc, server};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;

const HELP: &str = "canvas: vector painting server and CLI

usage:
  canvas [serve] [--port N] [--bind ADDR] [file.canvas] [--no-open]
  canvas stroke [--color #hex] [--width N] [--opacity 0..1] [--eraser] x,y x,y ...
  canvas clear | undo | redo | state | history
  canvas resize [--left N] [--top N] [--right N] [--bottom N]
  canvas new [--width N] [--height N] [--background #hex]
  canvas export out.svg|out.png|out.jpg [--scale N] [--quality N]
  canvas save file.canvas
  canvas load file.canvas
  canvas help

client commands talk to a running server: --url URL or CANVAS_URL
(default http://127.0.0.1:7878)
";

const BOOL_FLAGS: &[&str] = &["eraser", "no-open"];

struct Args {
    flags: HashMap<String, String>,
    bools: Vec<String>,
    pos: Vec<String>,
}

impl Args {
    fn parse(raw: &[String]) -> Result<Args, String> {
        let mut a = Args { flags: HashMap::new(), bools: Vec::new(), pos: Vec::new() };
        let mut i = 0;
        while i < raw.len() {
            let s = &raw[i];
            if let Some(name) = s.strip_prefix("--") {
                if let Some((k, v)) = name.split_once('=') {
                    a.flags.insert(k.into(), v.into());
                } else if BOOL_FLAGS.contains(&name) {
                    a.bools.push(name.into());
                } else {
                    let v = raw.get(i + 1).ok_or(format!("--{name} needs a value"))?;
                    a.flags.insert(name.into(), v.clone());
                    i += 1;
                }
            } else {
                a.pos.push(s.clone());
            }
            i += 1;
        }
        Ok(a)
    }
    fn has(&self, k: &str) -> bool {
        self.bools.iter().any(|b| b == k)
    }
    fn num(&self, k: &str) -> Result<Option<f64>, String> {
        self.flags.get(k).map(|v| v.parse::<f64>().map_err(|_| format!("--{k}: not a number: {v}"))).transpose()
    }
}

fn base_url(a: &Args) -> String {
    a.flags
        .get("url")
        .cloned()
        .or_else(|| std::env::var("CANVAS_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:7878".into())
        .trim_end_matches('/')
        .to_string()
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => format!("HTTP {code}: {}", r.into_string().unwrap_or_default()),
        e => e.to_string(),
    }
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let r = ureq::get(url).call().map_err(http_err)?;
    let mut b = Vec::new();
    r.into_reader().read_to_end(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
}

fn get_json(url: &str) -> Result<Value, String> {
    serde_json::from_slice(&get_bytes(url)?).map_err(|e| e.to_string())
}

fn post_json(url: &str, body: &Value) -> Result<Value, String> {
    let r = ureq::post(url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(http_err)?;
    serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn post_bytes(url: &str, body: &[u8]) -> Result<Value, String> {
    let r = ureq::post(url)
        .set("Content-Type", "application/octet-stream")
        .send_bytes(body)
        .map_err(http_err)?;
    serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn print(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

pub fn run(raw: Vec<String>) -> Result<(), String> {
    let (cmd, rest) = match raw.first() {
        None => ("serve", &raw[..]),
        Some(c) if c.starts_with("--") || c.ends_with(".canvas") => ("serve", &raw[..]),
        Some(c) => (c.as_str(), &raw[1..]),
    };
    let a = Args::parse(rest)?;
    let base = base_url(&a);
    let act = |v: Value| -> Result<(), String> { print(&post_json(&format!("{base}/api/actions"), &v)?); Ok(()) };
    match cmd {
        "help" | "-h" | "--help" => print!("{HELP}"),
        "serve" => {
            let port: u16 = match a.flags.get("port") {
                Some(p) => p.parse().map_err(|_| format!("bad port {p}"))?,
                None => 7878,
            };
            let bind = a.flags.get("bind").cloned().unwrap_or_else(|| "127.0.0.1".into());
            let doc = match a.pos.first() {
                Some(f) => format::decode(&std::fs::read(f).map_err(|e| format!("{f}: {e}"))?)?,
                None => Doc::new(1600, 1000, [255, 255, 255]),
            };
            if !a.has("no-open") {
                let host = if bind == "0.0.0.0" { "127.0.0.1" } else { bind.as_str() };
                let url = format!("http://{host}:{port}");
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    let _ = std::process::Command::new("open").arg(url).status();
                });
            }
            server::serve(&bind, port, doc)?;
        }
        "stroke" => {
            let mut pts = Vec::new();
            for p in &a.pos {
                let (x, y) = p.split_once(',').ok_or(format!("bad point {p}, expected x,y"))?;
                let x: f64 = x.trim().parse().map_err(|_| format!("bad point {p}"))?;
                let y: f64 = y.trim().parse().map_err(|_| format!("bad point {p}"))?;
                pts.push([x, y]);
            }
            if pts.is_empty() {
                return Err("stroke needs at least one x,y point".into());
            }
            act(json!({
                "type": "stroke",
                "color": a.flags.get("color").cloned().unwrap_or_else(|| "#000000".into()),
                "width": a.num("width")?.unwrap_or(4.0),
                "points": pts,
                "eraser": a.has("eraser"),
                "opacity": a.num("opacity")?.unwrap_or(1.0),
            }))?;
        }
        "clear" | "undo" | "redo" => act(json!({"type": cmd}))?,
        "resize" => act(json!({
            "type": "resize",
            "left": a.num("left")?.unwrap_or(0.0),
            "top": a.num("top")?.unwrap_or(0.0),
            "right": a.num("right")?.unwrap_or(0.0),
            "bottom": a.num("bottom")?.unwrap_or(0.0),
        }))?,
        "state" => print(&get_json(&format!("{base}/api/state"))?),
        "history" => {
            let v = get_json(&format!("{base}/api/actions?since=0"))?;
            for x in v["actions"].as_array().cloned().unwrap_or_default() {
                println!("{x}");
            }
        }
        "new" => {
            let mut body = json!({});
            if let Some(w) = a.num("width")? {
                body["width"] = json!(w);
            }
            if let Some(h) = a.num("height")? {
                body["height"] = json!(h);
            }
            if let Some(bg) = a.flags.get("background") {
                body["background"] = json!(bg);
            }
            print(&post_json(&format!("{base}/api/new"), &body)?);
        }
        "export" => {
            let out = a.pos.first().ok_or("export needs an output path")?;
            let ext = out.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            let ext = match ext.as_str() {
                "svg" | "png" => ext,
                "jpg" | "jpeg" => "jpg".into(),
                _ => return Err("output must end in .svg, .png or .jpg".into()),
            };
            let mut url = format!("{base}/api/export.{ext}?scale={}", a.num("scale")?.unwrap_or(1.0));
            if let Some(q) = a.num("quality")? {
                url.push_str(&format!("&quality={q}"));
            }
            let b = get_bytes(&url)?;
            std::fs::write(out, &b).map_err(|e| format!("{out}: {e}"))?;
            println!("wrote {out} ({} bytes)", b.len());
        }
        "save" => {
            let out = a.pos.first().ok_or("save needs a file path")?;
            let b = get_bytes(&format!("{base}/api/project"))?;
            std::fs::write(out, &b).map_err(|e| format!("{out}: {e}"))?;
            println!("wrote {out} ({} bytes)", b.len());
        }
        "load" => {
            let f = a.pos.first().ok_or("load needs a file path")?;
            let b = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
            format::decode(&b)?;
            print(&post_bytes(&format!("{base}/api/project"), &b)?);
        }
        other => return Err(format!("unknown command {other}, try `canvas help`")),
    }
    Ok(())
}
