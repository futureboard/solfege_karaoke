//! Web UI server: REST + WebSocket on top of axum, serving the Vite/React
//! build from `webui/dist`. The TUI thread stays the single source of
//! truth: it publishes a JSON snapshot into `WebHub` and receives
//! `WebAction`s back; meters are read straight from the engine atomics.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::services::{ServeDir, ServeFile};

use crate::engine::mixer::{FxParams, MAX_BUSES, MAX_STRIPS, StripParams};
use crate::engine::{MAX_SLOTS, Shared, load_peak};
use crate::instrument::{self, Instrument};
use crate::smf;

/// Partial update of a slot's parameters; absent fields stay unchanged.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SlotPatch {
    pub volume_db: Option<f32>,
    pub pan: Option<f32>,
    pub channels: Option<u16>,
    pub transpose: Option<i32>,
    pub tune: Option<f32>,
    pub key_lo: Option<u8>,
    pub key_hi: Option<u8>,
    pub vel_lo: Option<u8>,
    pub vel_hi: Option<u8>,
    pub bend_range: Option<f32>,
    pub mute: Option<bool>,
    pub solo: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WavPatch {
    pub root: Option<u8>,
    pub keytrack: Option<bool>,
    pub loop_mode: Option<String>,
    pub attack: Option<f32>,
    pub hold: Option<f32>,
    pub decay: Option<f32>,
    pub sustain: Option<f32>,
    pub release: Option<f32>,
}

/// Commands from the browser. Slots are addressed by their stable id.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebAction {
    AddInstrument { path: String },
    ReplaceInstrument { slot: u64, path: String },
    RemoveSlot { slot: u64 },
    SetSlot { slot: u64, patch: SlotPatch },
    SetWav { slot: u64, patch: WavPatch },
    SetPreset { slot: u64, preset: usize },
    SetChannelPreset { slot: u64, channel: u8, preset: Option<usize> },
    SetStrip { slot: u64, strip: usize, params: StripParams },
    SetNoteGroups { slot: u64, enabled: bool, map: Vec<u8>, names: Vec<String> },
    SetFx { fx: FxParams },
    SetMaster { db: f32 },
    Transport { op: String },
    Seek { time: f64 },
    SetLoop { on: bool },
    SetSpeed { speed: f64 },
    SetMutes { mutes: u16 },
    SetForward { on: bool },
    LoadSong { path: String },
    Note { slot: u64, key: u8, vel: u8 },
    Panic,
}

/// Slot id + instrument, so the web thread can answer preset queries and
/// label meters without asking the TUI thread.
#[derive(Clone)]
pub struct SlotMeta {
    pub id: u64,
    pub inst: Arc<Instrument>,
}

pub struct WebHub {
    state: RwLock<Arc<String>>,
    version: AtomicU64,
    slots: RwLock<Vec<SlotMeta>>,
    browse_dir: RwLock<PathBuf>,
    actions: Sender<WebAction>,
    shared: Arc<Shared>,
    pub clients: AtomicUsize,
}

impl WebHub {
    pub fn new(actions: Sender<WebAction>, shared: Arc<Shared>) -> Self {
        Self {
            state: RwLock::new(Arc::new("{}".into())),
            version: AtomicU64::new(0),
            slots: RwLock::new(Vec::new()),
            browse_dir: RwLock::new(PathBuf::from(".")),
            actions,
            shared,
            clients: AtomicUsize::new(0),
        }
    }

    /// Publish a new snapshot; bumps the version only when it changed.
    pub fn publish(&self, json: String, slots: Vec<SlotMeta>, browse_dir: &Path) {
        let changed = self.state.read().map(|s| **s != json).unwrap_or(true);
        if changed {
            if let Ok(mut s) = self.state.write() {
                *s = Arc::new(json);
            }
            if let Ok(mut m) = self.slots.write() {
                *m = slots;
            }
            self.version.fetch_add(1, Ordering::Relaxed);
        }
        if let Ok(mut d) = self.browse_dir.write()
            && *d != browse_dir
        {
            *d = browse_dir.to_path_buf();
        }
    }

    fn meters(&self) -> Value {
        let sh = &self.shared;
        let pair = |l: &std::sync::atomic::AtomicU32, r: &std::sync::atomic::AtomicU32| [load_peak(l), load_peak(r)];
        let slots: Vec<Value> = self
            .slots
            .read()
            .map(|v| v.clone())
            .unwrap_or_default()
            .iter()
            .enumerate()
            .take(MAX_SLOTS)
            .map(|(i, meta)| {
                let m = &sh.slots[i];
                let strips: Vec<[f32; 2]> = (0..MAX_STRIPS).map(|k| pair(&m.strip_l[k], &m.strip_r[k])).collect();
                let channels: Vec<Value> = m
                    .channels
                    .iter()
                    .map(|c| {
                        let c = c.load();
                        json!({
                            "preset": c.preset, "program": c.program, "msb": c.bank_msb, "lsb": c.bank_lsb,
                            "volume": c.volume, "pan": c.pan, "expression": c.expression, "bend": c.bend,
                            "sustain": c.sustain, "drum": c.drum, "voices": c.voices,
                            "explicit": c.explicit, "fallback": c.fallback, "locked": c.locked,
                        })
                    })
                    .collect();
                json!({
                    "id": meta.id,
                    "peak": pair(&m.peak_l, &m.peak_r),
                    "voices": m.voices.load(Ordering::Relaxed),
                    "live": m.strips_live.load(Ordering::Relaxed),
                    "notes": [m.notes[0].load(Ordering::Relaxed).to_string(), m.notes[1].load(Ordering::Relaxed).to_string()],
                    "strips": strips,
                    "channels": channels,
                })
            })
            .collect();
        json!({
            "type": "meters",
            "version": self.version.load(Ordering::Relaxed),
            "cpu": f32::from_bits(sh.cpu.load(Ordering::Relaxed)),
            "voices": sh.voices.load(Ordering::Relaxed),
            "master": pair(&sh.master_l, &sh.master_r),
            "buses": (0..MAX_BUSES).map(|b| pair(&sh.bus_l[b], &sh.bus_r[b])).collect::<Vec<_>>(),
            "fx": [load_peak(&sh.fx_peak[0]), load_peak(&sh.fx_peak[1])],
            "out_pairs": sh.out_pairs.load(Ordering::Relaxed),
            "player": {
                "state": sh.player_state.load(Ordering::Relaxed),
                "time": f64::from_bits(sh.player_time.load(Ordering::Relaxed)),
            },
            "activity": sh.channel_activity.iter().map(load_peak).collect::<Vec<_>>(),
            "slots": slots,
        })
    }
}

type Hub = Arc<WebHub>;

/// Bind and serve on a background thread. Returns the bound address.
pub fn start(addr: SocketAddr, hub: Hub, web_dir: Option<PathBuf>) -> Result<SocketAddr> {
    let listener = std::net::TcpListener::bind(addr).with_context(|| format!("bind web UI on {addr}"))?;
    listener.set_nonblocking(true)?;
    let local = listener.local_addr()?;
    std::thread::Builder::new().name("web".into()).spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
            let _ = axum::serve(listener, router(hub, web_dir)).await;
        });
    })?;
    Ok(local)
}

fn router(hub: Hub, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/api/state", get(get_state))
        .route("/api/slots/{id}/presets", get(get_presets))
        .route("/api/browse", get(browse))
        .route("/api/action", post(post_action))
        .route("/ws", get(ws_upgrade))
        .with_state(hub);
    match web_dir.filter(|d| d.join("index.html").exists()) {
        Some(dir) => {
            let index = dir.join("index.html");
            api.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => api.fallback(get(no_build)),
    }
}

async fn no_build() -> Html<&'static str> {
    Html(
        "<!doctype html><meta charset=utf-8><title>simpletui</title>\
         <body style='font-family:system-ui;background:#111;color:#ddd;padding:2rem'>\
         <h1>simpletui web UI is not built</h1>\
         <p>Run:</p><pre>cd app/liveinst/webui\nnpm install\nnpm run build</pre>\
         <p>then reload. The API is live at <code>/api/state</code>.</p>",
    )
}

async fn get_state(State(hub): State<Hub>) -> Response {
    let body = hub.state.read().map(|s| s.clone()).unwrap_or_else(|_| Arc::new("{}".into()));
    let version = hub.version.load(Ordering::Relaxed);
    (
        [(header::CONTENT_TYPE, "application/json".to_string()), (header::ETAG, version.to_string())],
        (*body).clone(),
    )
        .into_response()
}

#[derive(Serialize)]
struct PresetView<'a> {
    index: usize,
    bank: u16,
    program: u8,
    name: &'a str,
    zones: usize,
}

async fn get_presets(State(hub): State<Hub>, AxPath(id): AxPath<u64>) -> Response {
    let meta = hub.slots.read().ok().and_then(|v| v.iter().find(|m| m.id == id).cloned());
    match meta {
        Some(m) => {
            let list: Vec<PresetView> = m
                .inst
                .presets
                .iter()
                .enumerate()
                .map(|(index, p)| PresetView { index, bank: p.bank, program: p.program, name: &p.name, zones: p.zones.len() })
                .collect();
            Json(list).into_response()
        }
        None => (StatusCode::NOT_FOUND, "no such slot").into_response(),
    }
}

#[derive(Deserialize)]
struct BrowseQuery {
    path: Option<String>,
    kind: Option<String>,
}

#[derive(Serialize)]
struct BrowseEntry {
    name: String,
    path: String,
    dir: bool,
}

async fn browse(State(hub): State<Hub>, Query(q): Query<BrowseQuery>) -> Response {
    let songs = q.kind.as_deref() == Some("song");
    let accepts = |p: &Path| if songs { smf::is_midi(p) } else { instrument::is_supported(p) };
    let dir = match q.path.filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => hub.browse_dir.read().map(|d| d.clone()).unwrap_or_else(|_| PathBuf::from(".")),
    };
    let dir = std::fs::canonicalize(&dir).map(crate::browser::strip_verbatim).unwrap_or(dir);
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{}: {e}", dir.display())).into_response(),
    };
    for e in read.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = path.is_dir();
        if is_dir || accepts(&path) {
            entries.push(BrowseEntry { name, path: path.display().to_string(), dir: is_dir });
        }
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let roots: Vec<String> = if cfg!(windows) {
        (b'A'..=b'Z').map(|c| format!("{}:\\", c as char)).filter(|r| Path::new(r).exists()).collect()
    } else {
        vec!["/".into()]
    };
    Json(json!({
        "dir": dir.display().to_string(),
        "parent": dir.parent().map(|p| p.display().to_string()),
        "roots": roots,
        "entries": entries,
    }))
    .into_response()
}

async fn post_action(State(hub): State<Hub>, Json(action): Json<WebAction>) -> Response {
    match hub.actions.send(action) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "app is shutting down").into_response(),
    }
}

async fn ws_upgrade(State(hub): State<Hub>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| ws_loop(socket, hub))
}

/// Push meters at ~25 Hz; accept actions as text frames too.
async fn ws_loop(mut socket: WebSocket, hub: Hub) {
    hub.clients.fetch_add(1, Ordering::Relaxed);
    let mut tick = tokio::time::interval(Duration::from_millis(40));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let msg = hub.meters().to_string();
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(action) = serde_json::from_str::<WebAction>(&text) {
                        let _ = hub.actions.send(action);
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
    hub.clients.fetch_sub(1, Ordering::Relaxed);
}

/// `webui/dist` (or `app/liveinst/webui/dist` from the workspace root) next
/// to the working directory or any ancestor of the exe.
pub fn find_web_dir() -> Option<PathBuf> {
    const DIRS: [&str; 2] = ["webui/dist", "app/liveinst/webui/dist"];
    let mut candidates: Vec<PathBuf> = DIRS.iter().map(PathBuf::from).collect();
    if let Ok(exe) = std::env::current_exe() {
        candidates.extend(exe.ancestors().skip(1).flat_map(|a| DIRS.map(|d| a.join(d))));
    }
    candidates.into_iter().find(|c| c.join("index.html").exists())
}
