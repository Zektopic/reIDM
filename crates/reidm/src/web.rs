//! Local web UI + JSON API (served on 127.0.0.1 only).
//!
//! A web page on the internet can reach localhost, so every request is checked:
//! the `Host` header must be our own address (blocks DNS rebinding), mutating calls need the
//! per-run token that is only embedded in the page we serve, and cross-origin `Origin`s are refused.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use reidm_core::{Manager, Request};
use serde::Deserialize;
use std::sync::Arc;

pub const UI_PORT: u16 = 61002;
const PAGE: &str = include_str!("../assets/ui.html");

#[derive(Clone)]
struct App {
    mgr: Manager,
    token: Arc<String>,
}

fn random_token() -> String {
    let mut b = [0u8; 16];
    // std has no CSPRNG; read the OS source directly.
    #[cfg(unix)]
    {
        use std::io::Read;
        let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    }
    #[cfg(windows)]
    {
        #[link(name = "advapi32")]
        extern "system" {
            #[link_name = "SystemFunction036"]
            fn RtlGenRandom(buf: *mut u8, len: u32) -> u8;
        }
        unsafe { RtlGenRandom(b.as_mut_ptr(), b.len() as u32) };
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn guard(app: &App, h: &HeaderMap, need_token: bool) -> Result<(), StatusCode> {
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if host != format!("127.0.0.1:{UI_PORT}") && host != format!("localhost:{UI_PORT}") {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(o) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if o != format!("http://{host}") {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    if need_token && h.get("x-reidm-token").and_then(|v| v.to_str().ok()) != Some(app.token.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

async fn index(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, false) {
        return c.into_response();
    }
    let mut r = Html(PAGE.replace("__TOKEN__", &app.token)).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r.headers_mut().insert("content-security-policy", "default-src 'self' 'unsafe-inline'; frame-ancestors 'none'".parse().unwrap());
    r
}

async fn list(State(app): State<App>, h: HeaderMap) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    let mut items = app.mgr.list();
    for i in &mut items {
        i.request.cookie = None; // never expose session cookies to the page
    }
    Json(items).into_response()
}

#[derive(Deserialize)]
struct AddBody {
    url: String,
    referer: Option<String>,
    filename: Option<String>,
}

async fn add(State(app): State<App>, h: HeaderMap, Json(b): Json<AddBody>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match app.mgr.add(Request { url: b.url.trim().into(), referer: b.referer, filename: b.filename, ..Default::default() }) {
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

async fn act(State(app): State<App>, h: HeaderMap, Path((id, what)): Path<(u64, String)>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    match what.as_str() {
        "pause" => app.mgr.pause(id),
        "resume" => {
            if let Err(e) = app.mgr.resume(id) {
                return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response();
            }
        }
        "open" | "folder" => {
            let Some(it) = app.mgr.get(id) else { return StatusCode::NOT_FOUND.into_response() };
            crate::launch::open_path(&it.path, what == "folder");
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct Purge {
    #[serde(default)]
    purge: bool,
}

async fn remove(State(app): State<App>, h: HeaderMap, Path(id): Path<u64>, Query(q): Query<Purge>) -> Response {
    if let Err(c) = guard(&app, &h, true) {
        return c.into_response();
    }
    app.mgr.remove(id, q.purge);
    StatusCode::NO_CONTENT.into_response()
}

pub async fn serve(mgr: Manager) -> anyhow::Result<()> {
    let app = App { mgr, token: Arc::new(random_token()) };
    let router = Router::new()
        .route("/", get(index))
        .route("/api/downloads", get(list).post(add))
        .route("/api/downloads/:id", delete(remove))
        .route("/api/downloads/:id/:what", post(act))
        .with_state(app);
    let l = tokio::net::TcpListener::bind(("127.0.0.1", UI_PORT)).await?;
    tracing::info!("web UI at http://127.0.0.1:{UI_PORT}/");
    axum::serve(l, router).await?;
    Ok(())
}
