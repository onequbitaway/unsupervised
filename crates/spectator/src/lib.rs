//! Live feed for viewers.
//!
//! Viewers read the world through this crate. It can never change the
//! simulation: it only calls the read methods of [`SimHandle`].

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use unsup_protocol::{ViewQuery, ZoneId};
use unsup_runtime::SimHandle;

/// Returns the routes that viewers use.
pub fn router(sim: SimHandle) -> Router {
    Router::new()
        .route("/v1/world", get(layout))
        .route("/v1/world/zones/{zone}", get(zone))
        .route("/v1/spectate", get(spectate))
        .route("/debug", get(|| async { Html(DEBUG_PAGE) }))
        .with_state(sim)
}

async fn view(sim: &SimHandle, query: ViewQuery) -> Response {
    match sim.view(query).await {
        Ok(view) => Json(json!(view)).into_response(),
        Err(error) => {
            let body: Value =
                json!({ "error": { "code": "not_found", "message": error.to_string() } });
            (StatusCode::NOT_FOUND, Json(body)).into_response()
        }
    }
}

async fn layout(State(sim): State<SimHandle>) -> Response {
    view(&sim, ViewQuery::Layout).await
}

async fn zone(State(sim): State<SimHandle>, Path(zone): Path<u32>) -> Response {
    view(&sim, ViewQuery::Zone { zone: ZoneId(zone) }).await
}

async fn spectate(State(sim): State<SimHandle>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| feed(socket, sim))
}

/// Sends every frame to one viewer until the viewer leaves. A viewer that
/// is too slow loses frames; it never slows the simulation.
async fn feed(mut socket: WebSocket, sim: SimHandle) {
    let mut frames = sim.frames();
    loop {
        match frames.recv().await {
            Ok(frame) => {
                let Ok(text) = serde_json::to_string(&*frame) else { continue };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

/// A page without a build step that draws every resident as a dot. It is a
/// development tool. The real viewer is in `web/`.
const DEBUG_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Unsupervised: debug view</title>
<style>body{margin:0;background:#111;color:#ccc;font:12px monospace}canvas{display:block}#s{position:fixed;left:8px;top:8px}</style>
<div id="s">connecting</div><canvas id="c"></canvas>
<script>
const c = document.getElementById('c'), g = c.getContext('2d'), s = document.getElementById('s');
const residents = new Map(); let bounds = null, frame = 0;
function fit() { c.width = innerWidth; c.height = innerHeight; } fit(); onresize = fit;
async function load() {
  const world = await (await fetch('/v1/world')).json();
  for (const z of world.zones) {
    const b = z.bounds;
    bounds = bounds ? { x0: Math.min(bounds.x0, b.x0), y0: Math.min(bounds.y0, b.y0), x1: Math.max(bounds.x1, b.x1), y1: Math.max(bounds.y1, b.y1) } : { ...b };
    const view = await (await fetch('/v1/world/zones/' + z.id)).json();
    for (const r of view.residents) residents.set(r.id, r.pos);
  }
  const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/v1/spectate');
  ws.onmessage = (m) => {
    const f = JSON.parse(m.data); frame = f.frame;
    for (const z of f.zones) {
      for (const r of z.entered || []) residents.set(r.id, r.pos);
      for (const [id, pos] of z.moved || []) residents.set(id, pos);
    }
  };
}
function draw() {
  g.fillStyle = '#111'; g.fillRect(0, 0, c.width, c.height);
  if (bounds) {
    const k = Math.min(c.width / (bounds.x1 - bounds.x0), c.height / (bounds.y1 - bounds.y0));
    g.fillStyle = '#6cf';
    for (const p of residents.values()) g.fillRect((p.x - bounds.x0) * k, (p.y - bounds.y0) * k, Math.max(2, k), Math.max(2, k));
  }
  s.textContent = 'frame ' + frame + ', residents ' + residents.size;
  requestAnimationFrame(draw);
}
load(); draw();
</script>
"#;
