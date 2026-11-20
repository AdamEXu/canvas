use crate::format;
use crate::model::{action_json, parse_color, parse_op, Action, Doc};
use crate::render;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response};

const INDEX: &str = include_str!("../web/index.html");

struct Reply {
    code: u16,
    ctype: &'static str,
    body: Vec<u8>,
    attachment: Option<&'static str>,
}

fn ok_json(v: Value) -> Reply {
    Reply { code: 200, ctype: "application/json", body: v.to_string().into_bytes(), attachment: None }
}

fn err(code: u16, msg: &str) -> Reply {
    Reply { code, ctype: "application/json", body: json!({"error": msg}).to_string().into_bytes(), attachment: None }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn serve(bind: &str, port: u16, doc: Doc) -> Result<(), String> {
    let addr = format!("{bind}:{port}");
    let server = Arc::new(tiny_http::Server::http(&addr).map_err(|e| format!("bind {addr}: {e}"))?);
    let state = Arc::new(Mutex::new(doc));
    eprintln!("canvas listening on http://{addr}");
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let s = server.clone();
            let st = state.clone();
            thread::spawn(move || {
                for req in s.incoming_requests() {
                    handle(req, &st);
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn handle(mut req: Request, st: &Mutex<Doc>) {
    let url = req.url().to_string();
    let (path, qs) = url.split_once('?').unwrap_or((&url, ""));
    let query: HashMap<&str, &str> = qs.split('&').filter_map(|kv| kv.split_once('=')).collect();
    let method = req.method().clone();
    let mut body = Vec::new();
    let _ = req.as_reader().read_to_end(&mut body);
    let r = if method == Method::Options {
        Reply { code: 204, ctype: "text/plain", body: Vec::new(), attachment: None }
    } else {
        route(&method, path, &query, &body, st)
    };
    let mut resp = Response::new(r.code.into(), Vec::new(), Cursor::new(r.body.clone()), Some(r.body.len()), None);
    let mut headers = vec![
        ("Content-Type", r.ctype.to_string()),
        ("Access-Control-Allow-Origin", "*".into()),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS".into()),
        ("Access-Control-Allow-Headers", "*".into()),
        ("Cache-Control", "no-store".into()),
    ];
    if let Some(name) = r.attachment {
        headers.push(("Content-Disposition", format!("attachment; filename=\"{name}\"")));
    }
    for (k, v) in headers {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(h);
        }
    }
    let _ = req.respond(resp);
}

fn route(method: &Method, path: &str, query: &HashMap<&str, &str>, body: &[u8], st: &Mutex<Doc>) -> Reply {
    let snapshot = || st.lock().unwrap().clone();
    let scale = query.get("scale").and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0);
    match (method, path) {
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            Reply { code: 200, ctype: "text/html; charset=utf-8", body: INDEX.as_bytes().to_vec(), attachment: None }
        }
        (Method::Get, "/api/state") => ok_json(st.lock().unwrap().state_json()),
        (Method::Get, "/api/actions") => {
            let since = query.get("since").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
            let d = st.lock().unwrap();
            let mut v = d.state_json();
            let acts: Vec<Value> = d.actions.iter().skip(since).map(action_json).collect();
            v["actions"] = Value::Array(acts);
            ok_json(v)
        }
        (Method::Post, "/api/actions") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => return err(400, &e.to_string()),
            };
            let items = match v {
                Value::Array(a) => a,
                other => vec![other],
            };
            let mut ops = Vec::with_capacity(items.len());
            for it in items {
                match parse_op(it) {
                    Ok(o) => ops.push(o),
                    Err(e) => return err(400, &e),
                }
            }
            let mut d = st.lock().unwrap();
            let t = now_ms();
            d.actions.extend(ops.into_iter().map(|op| Action { t, op }));
            ok_json(json!({"version": d.actions.len()}))
        }
        (Method::Post, "/api/new") => {
            let v: Value = if body.iter().all(|b| b.is_ascii_whitespace()) {
                json!({})
            } else {
                match serde_json::from_slice(body) {
                    Ok(v) => v,
                    Err(e) => return err(400, &e.to_string()),
                }
            };
            let w = v.get("width").and_then(Value::as_f64).unwrap_or(1600.0).round().clamp(1.0, 16384.0) as u32;
            let h = v.get("height").and_then(Value::as_f64).unwrap_or(1000.0).round().clamp(1.0, 16384.0) as u32;
            let bg = match v.get("background").and_then(Value::as_str) {
                Some(s) => match parse_color(s) {
                    Some(c) => c,
                    None => return err(400, "bad background"),
                },
                None => [255, 255, 255],
            };
            let mut d = st.lock().unwrap();
            let next = d.doc + 1;
            *d = Doc::new(w, h, bg);
            d.doc = next;
            ok_json(d.state_json())
        }
        (Method::Get, "/api/export.svg") => Reply {
            code: 200,
            ctype: "image/svg+xml",
            body: render::svg(&snapshot()).into_bytes(),
            attachment: None,
        },
        (Method::Get, "/api/export.png") => match render::png(&snapshot(), scale) {
            Ok(b) => Reply { code: 200, ctype: "image/png", body: b, attachment: None },
            Err(e) => err(400, &e),
        },
        (Method::Get, "/api/export.jpg") => {
            let quality = query.get("quality").and_then(|s| s.parse::<u8>().ok()).unwrap_or(90);
            match render::jpg(&snapshot(), scale, quality) {
                Ok(b) => Reply { code: 200, ctype: "image/jpeg", body: b, attachment: None },
                Err(e) => err(400, &e),
            }
        }
        (Method::Get, "/api/project") => Reply {
            code: 200,
            ctype: "application/octet-stream",
            body: format::encode(&snapshot()),
            attachment: Some("drawing.canvas"),
        },
        (Method::Post, "/api/project") => match format::decode(body) {
            Ok(mut nd) => {
                let mut d = st.lock().unwrap();
                nd.doc = d.doc + 1;
                *d = nd;
                ok_json(d.state_json())
            }
            Err(e) => err(400, &e),
        },
        _ => err(404, "not found"),
    }
}
