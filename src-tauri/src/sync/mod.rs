// SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Peer-to-peer LAN sync between Kanri devices.
//!
//! Every device runs the same code: an encrypted HTTP listener, mDNS
//! discovery and a background scheduler. Any paired device can sync with any
//! other (PC <-> phone, phone <-> tablet, ...); there is no central server.
//! Board data never leaves the local network.

mod blobs;
mod crypto;
mod discovery;
pub mod doc;
mod net;

use doc::{Clock, StoreData, SyncDoc};
use net::{CallError, DeviceInfo, PairCode, PairRequest, PairResponse, PeerInfo, SyncRequest, SyncResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

const STORE_FILE: &str = ".kanri.dat";
const AUTO_INTERVAL: Duration = Duration::from_secs(60);
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(1500);
const PAIRING_TTL_MS: u64 = 10 * 60 * 1000;
const MAX_ADDRS: usize = 6;
const LOG_LEN: usize = 60;

pub const EVENT_STATUS: &str = "sync://status";
pub const EVENT_DATA_CHANGED: &str = "sync://data-changed";
pub const EVENT_PAIRED: &str = "sync://paired";

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ---------------------------------------------------------------------------
// Persistent configuration

#[derive(Clone, Serialize, Deserialize)]
struct Group {
    id: String,
    key: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    device_id: String,
    device_name: String,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    group: Option<Group>,
    #[serde(default)]
    peers: BTreeMap<String, PeerInfo>,
}

fn yes() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
struct DocState {
    doc: SyncDoc,
    clock: Clock,
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn default_device_name<R: Runtime>(app: &AppHandle<R>) -> String {
    let _ = app;
    #[cfg(mobile)]
    {
        "Android device".to_string()
    }
    #[cfg(desktop)]
    {
        let h = tauri_plugin_os::hostname();
        if h.is_empty() { "Computer".to_string() } else { h }
    }
}

// ---------------------------------------------------------------------------
// Status reported to the UI

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeerStatus {
    id: String,
    name: String,
    addrs: Vec<String>,
    /// "ok" | "offline" | "error" | "unknown"
    state: String,
    last_ok: Option<u64>,
    last_attempt: Option<u64>,
    last_error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    t: u64,
    level: String,
    msg: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    enabled: bool,
    paired: bool,
    device_id: String,
    device_name: String,
    port: u16,
    addrs: Vec<String>,
    /// "disabled" | "unpaired" | "idle" | "syncing" | "offline" | "error"
    state: String,
    last_sync: Option<u64>,
    last_error: Option<String>,
    discovery_error: Option<String>,
    peers: Vec<PeerStatus>,
    log: Vec<LogEntry>,
}

#[derive(Default)]
struct RuntimeState {
    syncing: bool,
    last_sync: Option<u64>,
    last_error: Option<String>,
    offline: bool,
    discovery_error: Option<String>,
    peers: HashMap<String, PeerStatus>,
    log: VecDeque<LogEntry>,
}

struct Pairing {
    token: String,
    expires: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    code: String,
    qr_svg: String,
    expires_at: u64,
    addrs: Vec<String>,
}

enum Trigger {
    Manual(Sender<Result<String, String>>),
    Changed,
    Peer(String),
}

// ---------------------------------------------------------------------------

pub struct Sync<R: Runtime> {
    app: AppHandle<R>,
    dir: PathBuf,
    /// Kanri's store file, as passed to the store plugin. Relative paths
    /// resolve to the app data directory, exactly like the frontend's.
    store_path: PathBuf,
    blobs: blobs::Blobs,
    cfg: Mutex<Config>,
    data: Mutex<DocState>,
    rt: Mutex<RuntimeState>,
    pairing: Mutex<Option<Pairing>>,
    discovery: Mutex<Option<discovery::Discovery>>,
    port: Mutex<u16>,
    tx: Mutex<Sender<Trigger>>,
}

pub type SyncHandle<R> = Arc<Sync<R>>;

impl<R: Runtime> Sync<R> {
    fn cfg_path(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    fn doc_path(&self) -> PathBuf {
        self.dir.join("doc.json")
    }

    fn save_cfg(&self, cfg: &Config) {
        if let Err(e) = write_json_atomic(&self.cfg_path(), cfg) {
            self.log("error", format!("Saving sync settings failed: {e}"));
        }
    }

    fn log(&self, level: &str, msg: String) {
        match level {
            "error" => log::error!("[sync] {msg}"),
            "warn" => log::warn!("[sync] {msg}"),
            _ => log::info!("[sync] {msg}"),
        }
        let mut rt = self.rt.lock().unwrap();
        rt.log.push_front(LogEntry { t: now_ms(), level: level.into(), msg });
        rt.log.truncate(LOG_LEN);
    }

    fn me(&self) -> DeviceInfo {
        let cfg = self.cfg.lock().unwrap();
        let port = *self.port.lock().unwrap();
        DeviceInfo { id: cfg.device_id.clone(), name: cfg.device_name.clone(), port, addrs: net::local_addrs(port) }
    }

    fn group_key(&self) -> Option<crypto::Key> {
        let cfg = self.cfg.lock().unwrap();
        cfg.group.as_ref().and_then(|g| crypto::decode_key(&g.key).ok())
    }

    pub fn status(&self) -> Status {
        let cfg = self.cfg.lock().unwrap().clone();
        let port = *self.port.lock().unwrap();
        let rt = self.rt.lock().unwrap();
        let peers = cfg
            .peers
            .values()
            .map(|p| {
                let mut s = rt.peers.get(&p.id).cloned().unwrap_or_else(|| PeerStatus {
                    state: "unknown".into(),
                    ..Default::default()
                });
                s.id = p.id.clone();
                s.name = if p.name.is_empty() { "Unnamed device".into() } else { p.name.clone() };
                s.addrs = p.addrs.clone();
                s
            })
            .collect::<Vec<_>>();
        let state = if !cfg.enabled {
            "disabled"
        } else if cfg.group.is_none() || cfg.peers.is_empty() {
            "unpaired"
        } else if rt.syncing {
            "syncing"
        } else if rt.last_error.is_some() {
            "error"
        } else if rt.offline {
            "offline"
        } else {
            "idle"
        };
        Status {
            enabled: cfg.enabled,
            paired: cfg.group.is_some() && !cfg.peers.is_empty(),
            device_id: cfg.device_id,
            device_name: cfg.device_name,
            port,
            addrs: net::local_addrs(port),
            state: state.into(),
            last_sync: rt.last_sync,
            last_error: rt.last_error.clone(),
            discovery_error: rt.discovery_error.clone(),
            peers,
            log: rt.log.iter().cloned().collect(),
        }
    }

    fn emit_status(&self) {
        let _ = self.app.emit(EVENT_STATUS, self.status());
    }

    fn trigger(&self, t: Trigger) {
        let _ = self.tx.lock().unwrap().send(t);
    }

    // -- peers ---------------------------------------------------------------

    /// Records what we learned about a peer. `seen_at` is the address the
    /// peer was actually reached at, which is moved to the front.
    fn upsert_peer(&self, info: &PeerInfo, seen_at: Option<String>) {
        let mut cfg = self.cfg.lock().unwrap();
        if info.id.is_empty() || info.id == cfg.device_id {
            return;
        }
        let entry = cfg.peers.entry(info.id.clone()).or_insert_with(|| PeerInfo {
            id: info.id.clone(),
            ..Default::default()
        });
        let mut addrs: Vec<String> = seen_at.into_iter().collect();
        addrs.extend(info.addrs.iter().cloned());
        addrs.extend(entry.addrs.iter().cloned());
        let mut uniq = Vec::new();
        for a in addrs {
            if !uniq.contains(&a) {
                uniq.push(a);
            }
        }
        uniq.truncate(MAX_ADDRS);
        let changed = entry.addrs != uniq
            || (!info.name.is_empty() && info.last_seen >= entry.last_seen && entry.name != info.name);
        entry.addrs = uniq;
        if !info.name.is_empty() && (info.last_seen >= entry.last_seen || entry.name.is_empty()) {
            entry.name = info.name.clone();
        }
        entry.last_seen = entry.last_seen.max(info.last_seen);
        if changed {
            let snapshot = cfg.clone();
            drop(cfg);
            self.save_cfg(&snapshot);
        }
    }

    fn device_as_peer(d: &DeviceInfo) -> PeerInfo {
        PeerInfo { id: d.id.clone(), name: d.name.clone(), addrs: d.addrs.clone(), last_seen: now_ms() }
    }

    fn gossip(&self, peers: &[PeerInfo]) {
        for p in peers {
            self.upsert_peer(p, None);
        }
    }

    fn known_peers(&self) -> Vec<PeerInfo> {
        let cfg = self.cfg.lock().unwrap();
        let mut v: Vec<PeerInfo> = cfg.peers.values().cloned().collect();
        let port = *self.port.lock().unwrap();
        v.push(PeerInfo {
            id: cfg.device_id.clone(),
            name: cfg.device_name.clone(),
            addrs: net::local_addrs(port),
            last_seen: now_ms(),
        });
        v
    }

    fn set_peer_status(&self, id: &str, f: impl FnOnce(&mut PeerStatus)) {
        let mut rt = self.rt.lock().unwrap();
        let s = rt.peers.entry(id.to_string()).or_insert_with(|| PeerStatus {
            id: id.to_string(),
            state: "unknown".into(),
            ..Default::default()
        });
        f(s);
    }

    // -- store <-> document ----------------------------------------------------

    fn open_store(&self) -> Result<Arc<tauri_plugin_store::Store<R>>, String> {
        self.app
            .get_store(&self.store_path)
            .map(Ok)
            .unwrap_or_else(|| self.app.store(&self.store_path))
            .map_err(|e| format!("could not open board storage: {e}"))
    }

    fn read_store(&self) -> Result<StoreData, String> {
        let store = self.open_store()?;
        Ok(StoreData {
            boards: store.get("boards").unwrap_or(Value::Array(vec![])),
            pins: store.get("pins").unwrap_or(Value::Array(vec![])),
        })
    }

    fn write_store(&self, data: &StoreData) -> Result<(), String> {
        let store = self.open_store()?;
        store.set("boards", data.boards.clone());
        store.set("pins", data.pins.clone());
        store.save().map_err(|e| format!("could not save boards: {e}"))?;
        let _ = self.app.emit(EVENT_DATA_CHANGED, ());
        Ok(())
    }

    fn persist_doc(&self, st: &DocState) {
        if let Err(e) = write_json_atomic(&self.doc_path(), st) {
            self.log("error", format!("Saving sync state failed: {e}"));
        }
    }

    /// Captures local edits from the store into the replicated document.
    fn stamp_locked(&self, st: &mut DocState) -> Result<StoreData, String> {
        let mut data = self.read_store()?;
        if doc::normalize_ids(&mut data, || crypto::random_id(12)) {
            self.write_store(&data)?;
        }
        let mut portable = data.clone();
        self.blobs.to_portable(&mut portable);
        if st.doc.stamp(&portable, &mut st.clock, now_ms()) {
            self.persist_doc(st);
        }
        Ok(data)
    }

    /// Background images the current document uses that this device lacks.
    fn missing_blobs(&self) -> std::collections::BTreeSet<(String, String)> {
        let st = self.data.lock().unwrap();
        let rendered = st.doc.render(&StoreData::default());
        self.blobs.missing(rendered.boards.as_array().map(Vec::as_slice).unwrap_or(&[]))
    }

    /// Points boards at background images that have arrived since the last render.
    fn resolve_blob_paths(&self) -> Result<(), String> {
        let _st = self.data.lock().unwrap();
        let local = self.read_store()?;
        let mut updated = local.clone();
        self.blobs.to_local(&mut updated);
        if updated != local {
            self.write_store(&updated)?;
        }
        Ok(())
    }

    /// Downloads missing background images from a peer. Failures are only
    /// logged: the peer may not have the image either.
    fn fetch_blobs(&self, addr: &str, key: &crypto::Key) {
        let missing = self.missing_blobs();
        if missing.is_empty() {
            return;
        }
        let mut got = 0;
        for (hash, ext) in missing {
            let req = net::BlobRequest { hash: hash.clone(), ext: ext.clone() };
            match net::call::<_, net::BlobResponse>(addr, "/blob", key, &req) {
                Ok(resp) => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(resp.data).unwrap_or_default();
                    match self.blobs.write(&hash, &ext, &bytes) {
                        Ok(()) => got += 1,
                        Err(e) => self.log("warn", format!("Background image: {e}")),
                    }
                }
                Err(e) => log::info!("[sync] background {hash} not available from {addr}: {e}"),
            }
        }
        if got > 0 {
            if let Err(e) = self.resolve_blob_paths() {
                self.log("error", format!("Applying background images failed: {e}"));
            } else {
                self.log("info", format!("Received {got} background image(s)"));
            }
        }
    }

    fn stamp_local(&self) -> Result<String, String> {
        let mut st = self.data.lock().unwrap();
        self.stamp_locked(&mut st)?;
        Ok(st.doc.digest())
    }

    /// Merges a remote document and writes the result to the store.
    /// Returns the new digest and a copy of the merged document.
    fn apply_remote(&self, remote: &SyncDoc) -> Result<(String, SyncDoc, bool), String> {
        let mut st = self.data.lock().unwrap();
        let before = self.stamp_locked(&mut st)?;
        st.clock.observe(remote);
        let changed = st.doc.merge(remote);
        if changed {
            let mut after = st.doc.render(&before);
            self.blobs.to_local(&mut after);
            SyncDoc::touch_changed_boards(&before, &mut after, &now_iso());
            if after != before {
                self.write_store(&after)?;
            }
            self.persist_doc(&st);
        }
        Ok((st.doc.digest(), st.doc.clone(), changed))
    }

    fn snapshot(&self) -> Result<(String, SyncDoc), String> {
        let mut st = self.data.lock().unwrap();
        self.stamp_locked(&mut st)?;
        Ok((st.doc.digest(), st.doc.clone()))
    }

    // -- outgoing sync -----------------------------------------------------------

    fn sync_peer(&self, peer: &PeerInfo, key: &crypto::Key) -> Result<bool, CallError> {
        let digest = self.stamp_local().map_err(CallError::Failed)?;
        let me = self.me();
        let mut last_unreachable = String::from("no known address");
        for addr in &peer.addrs {
            let req = SyncRequest { from: me.clone(), peers: self.known_peers(), digest: digest.clone(), doc: None };
            match net::call::<_, SyncResponse>(addr, "/sync", key, &req) {
                Ok(resp) => {
                    if resp.from.id != peer.id {
                        // A different group member now lives at this address.
                        self.upsert_peer(&Self::device_as_peer(&resp.from), Some(addr.clone()));
                        last_unreachable = format!("{addr} is now used by another device");
                        continue;
                    }
                    self.upsert_peer(&Self::device_as_peer(&resp.from), Some(addr.clone()));
                    self.gossip(&resp.peers);
                    let mut changed = false;
                    if let Some(remote) = resp.doc {
                        let (new_digest, merged, ch) = self.apply_remote(&remote).map_err(CallError::Failed)?;
                        changed = ch;
                        if new_digest != resp.digest {
                            let push = SyncRequest {
                                from: me.clone(),
                                peers: self.known_peers(),
                                digest: new_digest,
                                doc: Some(merged),
                            };
                            net::call::<_, SyncResponse>(addr, "/sync", key, &push)?;
                        }
                    }
                    self.fetch_blobs(addr, key);
                    return Ok(changed);
                }
                Err(CallError::Unreachable(e)) => last_unreachable = format!("{addr}: {e}"),
                Err(e) => return Err(e),
            }
        }
        Err(CallError::Unreachable(last_unreachable))
    }

    fn run_all(&self, manual: bool) -> Result<String, String> {
        let (enabled, peers) = {
            let cfg = self.cfg.lock().unwrap();
            (cfg.enabled, cfg.peers.values().cloned().collect::<Vec<_>>())
        };
        if !enabled {
            return Err("Sync is turned off on this device.".into());
        }
        let Some(key) = self.group_key() else {
            return Err("This device is not paired yet. Open Sync settings to pair it with another device.".into());
        };
        if peers.is_empty() {
            return Err("No paired devices yet. Open Sync settings to pair another device.".into());
        }

        self.rt.lock().unwrap().syncing = true;
        self.emit_status();

        let mut ok = 0;
        let mut changed = false;
        let mut failures = Vec::new();
        for p in &peers {
            let start = now_ms();
            self.set_peer_status(&p.id, |s| s.last_attempt = Some(start));
            match self.sync_peer(p, &key) {
                Ok(ch) => {
                    ok += 1;
                    changed |= ch;
                    self.set_peer_status(&p.id, |s| {
                        s.state = "ok".into();
                        s.last_ok = Some(now_ms());
                        s.last_error = None;
                    });
                }
                Err(CallError::Unreachable(e)) => {
                    self.set_peer_status(&p.id, |s| s.state = "offline".into());
                    if manual {
                        self.log("info", format!("{} not reachable ({e})", display_name(p)));
                    }
                }
                Err(CallError::Failed(e)) => {
                    let msg = format!("{}: {e}", display_name(p));
                    self.log("error", format!("Sync with {msg}"));
                    self.set_peer_status(&p.id, |s| {
                        s.state = "error".into();
                        s.last_error = Some(e.clone());
                    });
                    failures.push(msg);
                }
            }
        }

        let result = {
            let mut rt = self.rt.lock().unwrap();
            rt.syncing = false;
            rt.offline = ok == 0 && failures.is_empty();
            if !failures.is_empty() {
                rt.last_error = Some(failures.join("\n"));
                Err(format!("Sync failed — {}", failures.join("; ")))
            } else if ok == 0 {
                rt.last_error = None;
                Err("No paired devices were reachable. Make sure they are on the same Wi-Fi with Kanri Sync open.".into())
            } else {
                rt.last_error = None;
                rt.last_sync = Some(now_ms());
                Ok(if changed {
                    format!("Synced with {ok} device(s); changes received.")
                } else {
                    format!("Synced with {ok} device(s); everything up to date.")
                })
            }
        };
        if changed {
            self.log("info", "Received changes from other devices".into());
        }
        self.emit_status();
        result
    }

    fn run_peer(&self, id: &str) {
        let peer = self.cfg.lock().unwrap().peers.get(id).cloned();
        let (Some(peer), Some(key)) = (peer, self.group_key()) else { return };
        if !self.cfg.lock().unwrap().enabled {
            return;
        }
        match self.sync_peer(&peer, &key) {
            Ok(_) => self.set_peer_status(id, |s| {
                s.state = "ok".into();
                s.last_ok = Some(now_ms());
                s.last_error = None;
            }),
            Err(CallError::Unreachable(_)) => self.set_peer_status(id, |s| s.state = "offline".into()),
            Err(CallError::Failed(e)) => {
                self.log("error", format!("Sync with {}: {e}", display_name(&peer)));
                self.set_peer_status(id, |s| {
                    s.state = "error".into();
                    s.last_error = Some(e);
                });
            }
        }
        self.emit_status();
    }

    // -- incoming requests -------------------------------------------------------

    fn handle(&self, inc: net::Incoming) -> (u16, Vec<u8>) {
        let result = match inc.path.as_str() {
            "/sync" => self.handle_sync(&inc),
            "/pair" => self.handle_pair(&inc),
            "/blob" => self.handle_blob(&inc),
            _ => Err((404, "not found".to_string())),
        };
        match result {
            Ok(body) => (200, body),
            Err((code, msg)) => (code, msg.into_bytes()),
        }
    }

    fn handle_sync(&self, inc: &net::Incoming) -> Result<Vec<u8>, (u16, String)> {
        if !self.cfg.lock().unwrap().enabled {
            return Err((503, "sync is turned off on that device.".to_string()));
        }
        let key = self
            .group_key()
            .ok_or((409, "that device is not paired any more. Pair the devices again.".to_string()))?;
        let req: SyncRequest = crypto::open(&key, "/sync", &inc.body)
            .map_err(|_| (401, "that device belongs to a different sync group. Pair the devices again.".to_string()))?;
        let seen_at = inc.remote_ip.map(|ip| format!("{ip}:{}", req.from.port));
        self.upsert_peer(&Self::device_as_peer(&req.from), seen_at);
        self.gossip(&req.peers);

        let (digest, doc, changed) = match &req.doc {
            Some(remote) => self.apply_remote(remote).map_err(|e| (500, e))?,
            None => {
                let (d, doc) = self.snapshot().map_err(|e| (500, e))?;
                (d, doc, false)
            }
        };
        let out_doc = if req.doc.is_none() && digest != req.digest { Some(doc) } else { None };
        self.set_peer_status(&req.from.id, |s| {
            s.state = "ok".into();
            s.last_ok = Some(now_ms());
            s.last_error = None;
        });
        {
            let mut rt = self.rt.lock().unwrap();
            rt.last_sync = Some(now_ms());
            rt.offline = false;
        }
        if changed {
            self.log("info", format!("Received changes from {}", req.from.name));
        }
        if !self.missing_blobs().is_empty() {
            // Fetch the images by syncing back with the sender shortly.
            self.trigger(Trigger::Peer(req.from.id.clone()));
        }
        self.emit_status();
        let resp = SyncResponse { from: self.me(), peers: self.known_peers(), digest, doc: out_doc };
        crypto::seal(&key, "/sync", &resp).map_err(|e| (500, e))
    }

    fn handle_blob(&self, inc: &net::Incoming) -> Result<Vec<u8>, (u16, String)> {
        let key = self.group_key().ok_or((409, "not paired".to_string()))?;
        let req: net::BlobRequest =
            crypto::open(&key, "/blob", &inc.body).map_err(|_| (401, "not in this sync group".to_string()))?;
        let bytes = self.blobs.read(&req.hash, &req.ext).ok_or((404, "image not found".to_string()))?;
        use base64::Engine;
        let resp = net::BlobResponse { data: base64::engine::general_purpose::STANDARD.encode(bytes) };
        crypto::seal(&key, "/blob", &resp).map_err(|e| (500, e))
    }

    fn handle_pair(&self, inc: &net::Incoming) -> Result<Vec<u8>, (u16, String)> {
        let token = {
            let p = self.pairing.lock().unwrap();
            match p.as_ref() {
                Some(p) if p.expires > now_ms() => p.token.clone(),
                _ => return Err((410, "Pairing is not open on that device (the code expired or the dialog was closed).".into())),
            }
        };
        let pk = crypto::pairing_key(&token);
        let req: PairRequest =
            crypto::open(&pk, "/pair", &inc.body).map_err(|_| (401, "Pairing code does not match.".to_string()))?;
        let group = self.ensure_group();
        let seen_at = inc.remote_ip.map(|ip| format!("{ip}:{}", req.device.port));
        self.upsert_peer(&Self::device_as_peer(&req.device), seen_at);
        let resp = PairResponse {
            group_id: group.id,
            group_key: group.key,
            from: self.me(),
            peers: self.known_peers(),
        };
        self.log("info", format!("Paired with {}", req.device.name));
        let _ = self.app.emit(EVENT_PAIRED, req.device.name.clone());
        self.emit_status();
        crypto::seal(&pk, "/pair", &resp).map_err(|e| (500, e))
    }

    fn ensure_group(&self) -> Group {
        let mut cfg = self.cfg.lock().unwrap();
        if let Some(g) = &cfg.group {
            return g.clone();
        }
        let g = Group { id: crypto::random_id(16), key: crypto::encode_key(&crypto::random_key()) };
        cfg.group = Some(g.clone());
        let snapshot = cfg.clone();
        drop(cfg);
        self.save_cfg(&snapshot);
        self.restart_discovery();
        g
    }

    fn restart_discovery(&self) {
        let mut d = self.discovery.lock().unwrap();
        *d = None;
        let (device_id, group) = {
            let cfg = self.cfg.lock().unwrap();
            (cfg.device_id.clone(), cfg.group.clone())
        };
        let Some(group) = group else { return };
        let port = *self.port.lock().unwrap();
        let app = self.app.clone();
        let res = discovery::Discovery::start(&device_id, &crypto::group_tag(&group.id), port, move |found| {
            if let Some(sync) = app.try_state::<SyncHandle<R>>() {
                let known = sync.cfg.lock().unwrap().peers.get(&found.device_id).cloned();
                let first = found.addrs.first().cloned();
                let info = PeerInfo {
                    id: found.device_id.clone(),
                    name: known.as_ref().map(|k| k.name.clone()).unwrap_or_default(),
                    addrs: found.addrs,
                    last_seen: known.map(|k| k.last_seen).unwrap_or(0),
                };
                sync.upsert_peer(&info, first);
                sync.trigger(Trigger::Peer(found.device_id));
            }
        });
        match res {
            Ok(disc) => {
                *d = Some(disc);
                self.rt.lock().unwrap().discovery_error = None;
            }
            Err(e) => {
                drop(d);
                self.log("warn", format!("Automatic discovery unavailable: {e}. Known addresses will still be used."));
                self.rt.lock().unwrap().discovery_error = Some(e);
            }
        }
    }

    // -- pairing -----------------------------------------------------------------

    fn start_pairing(&self) -> Result<PairingInfo, String> {
        self.ensure_group();
        let token = crypto::random_id(16);
        let expires = now_ms() + PAIRING_TTL_MS;
        *self.pairing.lock().unwrap() = Some(Pairing { token: token.clone(), expires });
        let me = self.me();
        if me.addrs.is_empty() {
            return Err("This device does not seem to be connected to a network.".into());
        }
        let code = PairCode { a: me.addrs.clone(), t: token, n: me.name }.encode();
        Ok(PairingInfo { qr_svg: qr_svg(&code), code, expires_at: expires, addrs: me.addrs })
    }

    /// Joins the group of the device that displayed `code`. Returns its name.
    fn join(&self, code: &str) -> Result<String, String> {
        let pc = PairCode::decode(code)?;
        let pk = crypto::pairing_key(&pc.t);
        let req = PairRequest { device: self.me() };
        let mut last = format!("Could not reach {}. Make sure both devices are on the same Wi-Fi.", pc.n);
        for addr in &pc.a {
            match net::call::<_, PairResponse>(addr, "/pair", &pk, &req) {
                Ok(resp) => {
                    crypto::decode_key(&resp.group_key)?;
                    let snapshot = {
                        let mut cfg = self.cfg.lock().unwrap();
                        cfg.group = Some(Group { id: resp.group_id, key: resp.group_key });
                        cfg.enabled = true;
                        cfg.clone()
                    };
                    self.save_cfg(&snapshot);
                    self.upsert_peer(&Self::device_as_peer(&resp.from), Some(addr.clone()));
                    self.gossip(&resp.peers);
                    self.restart_discovery();
                    self.log("info", format!("Joined sync group via {}", resp.from.name));
                    self.emit_status();
                    let (tx, _rx) = mpsc::channel();
                    self.trigger(Trigger::Manual(tx));
                    return Ok(resp.from.name);
                }
                Err(CallError::Unreachable(_)) => continue,
                Err(CallError::Failed(e)) => {
                    last = e;
                    break;
                }
            }
        }
        self.log("error", format!("Pairing failed: {last}"));
        self.emit_status();
        Err(last)
    }

    // -- scheduler ---------------------------------------------------------------

    fn scheduler(self: Arc<Self>, rx: Receiver<Trigger>) {
        let mut next_auto = Instant::now() + Duration::from_secs(3);
        let mut change_due: Option<Instant> = None;
        loop {
            let now = Instant::now();
            let due = change_due.map_or(next_auto, |c| c.min(next_auto));
            let wait = due.saturating_duration_since(now);
            match rx.recv_timeout(wait) {
                Ok(Trigger::Changed) => {
                    change_due = Some(Instant::now() + CHANGE_DEBOUNCE);
                }
                Ok(Trigger::Manual(reply)) => {
                    let r = self.run_all(true);
                    if let Err(e) = &r {
                        let mut rt = self.rt.lock().unwrap();
                        if rt.last_error.is_none() && !e.starts_with("No paired devices were reachable") {
                            rt.last_error = Some(e.clone());
                        }
                    }
                    let _ = reply.send(r);
                    change_due = None;
                    next_auto = Instant::now() + AUTO_INTERVAL;
                }
                Ok(Trigger::Peer(id)) => self.run_peer(&id),
                Err(RecvTimeoutError::Timeout) => {
                    let paired = {
                        let cfg = self.cfg.lock().unwrap();
                        cfg.enabled && cfg.group.is_some() && !cfg.peers.is_empty()
                    };
                    if paired {
                        let _ = self.run_all(false);
                    }
                    change_due = None;
                    next_auto = Instant::now() + AUTO_INTERVAL;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

fn display_name(p: &PeerInfo) -> String {
    if p.name.is_empty() { "Unnamed device".into() } else { p.name.clone() }
}

fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
        Err(_) => String::new(),
    }
}

/// On first launch, copy boards from an existing upstream Kanri install
/// (same machine, different app identifier) so nothing has to be re-entered.
#[cfg(desktop)]
fn migrate_from_upstream<R: Runtime>(app: &AppHandle<R>) {
    let Ok(dir) = app.path().app_data_dir() else { return };
    let ours = dir.join(STORE_FILE);
    if ours.exists() {
        return;
    }
    let Some(parent) = dir.parent() else { return };
    let theirs = parent.join("tech.trobonox.kanri").join(STORE_FILE);
    if theirs.exists() {
        let _ = std::fs::create_dir_all(&dir);
        match std::fs::copy(&theirs, &ours) {
            Ok(_) => log::info!("[sync] imported boards from existing Kanri installation"),
            Err(e) => log::warn!("[sync] could not import existing Kanri boards: {e}"),
        }
    }
}

pub fn init<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    #[cfg(desktop)]
    migrate_from_upstream(app);

    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let sync = start(app, base.join("sync"), PathBuf::from(STORE_FILE))?;
    app.manage(sync);
    Ok(())
}

/// Starts sync with state in `dir`, syncing the store at `store_path`.
fn start<R: Runtime>(app: &AppHandle<R>, dir: PathBuf, store_path: PathBuf) -> Result<SyncHandle<R>, String> {
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let cfg: Config = read_json(&dir.join("config.json")).unwrap_or_else(|| Config {
        device_id: crypto::random_id(12),
        device_name: default_device_name(app),
        enabled: true,
        group: None,
        peers: BTreeMap::new(),
    });
    let mut data: DocState = read_json(&dir.join("doc.json")).unwrap_or_default();
    data.clock.device = cfg.device_id.clone();

    let (tx, rx) = mpsc::channel();
    let sync = Arc::new(Sync {
        app: app.clone(),
        blobs: blobs::Blobs::new(dir.join("blobs")),
        dir,
        store_path,
        cfg: Mutex::new(cfg.clone()),
        data: Mutex::new(data),
        rt: Mutex::new(RuntimeState::default()),
        pairing: Mutex::new(None),
        discovery: Mutex::new(None),
        port: Mutex::new(net::DEFAULT_PORT),
        tx: Mutex::new(tx),
    });
    sync.save_cfg(&cfg);

    let handler = sync.clone();
    match net::serve(net::DEFAULT_PORT, move |inc| handler.handle(inc)) {
        Ok(port) => *sync.port.lock().unwrap() = port,
        Err(e) => sync.log("error", e),
    }
    sync.restart_discovery();

    let worker = sync.clone();
    std::thread::Builder::new()
        .name("kanri-sync-scheduler".into())
        .spawn(move || worker.scheduler(rx))
        .map_err(|e| e.to_string())?;
    Ok(sync)
}

// ---------------------------------------------------------------------------
// Commands

type Cmd<T> = Result<T, String>;

/// Commands run on the real (Wry) runtime; tests drive [`Sync`] directly.
type AppSync = SyncHandle<tauri::Wry>;

#[tauri::command]
pub fn sync_status(sync: tauri::State<'_, AppSync>) -> Status {
    sync.status()
}

#[tauri::command]
pub async fn sync_now(sync: tauri::State<'_, AppSync>) -> Cmd<String> {
    let (tx, rx) = mpsc::channel();
    sync.trigger(Trigger::Manual(tx));
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(120)).unwrap_or_else(|_| Err("Sync timed out.".into()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Called by the UI after it saved boards, so edits get accurate timestamps
/// and are pushed to other devices shortly after.
#[tauri::command]
pub async fn sync_local_change(sync: tauri::State<'_, AppSync>) -> Cmd<()> {
    let s = sync.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = s.stamp_local().map(|_| ());
        s.trigger(Trigger::Changed);
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn sync_start_pairing(sync: tauri::State<'_, AppSync>) -> Cmd<PairingInfo> {
    sync.start_pairing()
}

#[tauri::command]
pub fn sync_stop_pairing(sync: tauri::State<'_, AppSync>) {
    *sync.pairing.lock().unwrap() = None;
}

#[tauri::command]
pub async fn sync_join(sync: tauri::State<'_, AppSync>, code: String) -> Cmd<String> {
    let s = sync.inner().clone();
    tauri::async_runtime::spawn_blocking(move || s.join(&code))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn sync_rename_device(sync: tauri::State<'_, AppSync>, name: String) -> Cmd<()> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Name cannot be empty.".into());
    }
    let snapshot = {
        let mut cfg = sync.cfg.lock().unwrap();
        cfg.device_name = name;
        cfg.clone()
    };
    sync.save_cfg(&snapshot);
    sync.emit_status();
    Ok(())
}

#[tauri::command]
pub fn sync_set_enabled(sync: tauri::State<'_, AppSync>, enabled: bool) {
    let snapshot = {
        let mut cfg = sync.cfg.lock().unwrap();
        cfg.enabled = enabled;
        cfg.clone()
    };
    sync.save_cfg(&snapshot);
    if enabled {
        sync.restart_discovery();
    } else {
        *sync.discovery.lock().unwrap() = None;
    }
    sync.emit_status();
}

#[tauri::command]
pub fn sync_forget_device(sync: tauri::State<'_, AppSync>, id: String) {
    let snapshot = {
        let mut cfg = sync.cfg.lock().unwrap();
        cfg.peers.remove(&id);
        cfg.clone()
    };
    sync.rt.lock().unwrap().peers.remove(&id);
    sync.save_cfg(&snapshot);
    sync.emit_status();
}

/// Leaves the sync group. Boards stay on this device; it simply stops syncing.
#[tauri::command]
pub fn sync_leave_group(sync: tauri::State<'_, AppSync>) {
    let snapshot = {
        let mut cfg = sync.cfg.lock().unwrap();
        cfg.group = None;
        cfg.peers.clear();
        cfg.clone()
    };
    *sync.discovery.lock().unwrap() = None;
    *sync.pairing.lock().unwrap() = None;
    {
        let mut rt = sync.rt.lock().unwrap();
        rt.peers.clear();
        rt.last_error = None;
        rt.offline = false;
    }
    sync.save_cfg(&snapshot);
    sync.log("info", "Left the sync group".into());
    sync.emit_status();
}

#[tauri::command]
pub fn sync_clear_error(sync: tauri::State<'_, AppSync>) {
    sync.rt.lock().unwrap().last_error = None;
    sync.emit_status();
}

#[cfg(test)]
mod e2e {
    //! Two (or three) complete sync stacks in one process talking over real
    //! sockets: pairing, bidirectional sync, and rejection of strangers.
    use super::*;
    use serde_json::json;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    struct Node {
        _app: tauri::App<MockRuntime>,
        sync: SyncHandle<MockRuntime>,
    }

    fn node(name: &str, boards: Value) -> Node {
        let app = mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(mock_context(noop_assets()))
            .unwrap();
        let dir = std::env::temp_dir().join(format!("kanri-sync-e2e-{}", crypto::random_id(6)));
        let sync = start(app.handle(), dir.join("sync"), dir.join("kanri.dat")).unwrap();
        sync.cfg.lock().unwrap().device_name = name.into();
        sync.write_store(&StoreData { boards, pins: json!([]) }).unwrap();
        Node { _app: app, sync }
    }

    fn board(id: &str, card: &str) -> Value {
        json!({ "id": id, "title": id, "columns": [
            { "id": format!("{id}-col"), "title": "Todo", "cards": [{ "id": format!("{id}-card"), "name": card }] }
        ]})
    }

    fn titles(n: &Node) -> Vec<String> {
        let mut t: Vec<String> = n.sync.read_store().unwrap().boards.as_array().unwrap().iter()
            .map(|b| b["title"].as_str().unwrap().to_string()).collect();
        t.sort();
        t
    }

    fn card_name(n: &Node, board_id: &str) -> String {
        let d = n.sync.read_store().unwrap();
        let b = d.boards.as_array().unwrap().iter().find(|b| b["id"] == board_id).unwrap().clone();
        b["columns"][0]["cards"][0]["name"].as_str().unwrap().to_string()
    }

    #[test]
    fn pair_and_sync_both_directions() {
        let pc = node("PC", json!([board("work", "write report")]));
        let phone = node("Phone", json!([board("home", "buy milk")]));

        let code = pc.sync.start_pairing().unwrap().code;
        assert_eq!(phone.sync.join(&code).unwrap(), "PC");

        // Phone pulls the PC's board and pushes its own.
        phone.sync.run_all(true).unwrap();
        assert_eq!(titles(&pc), vec!["home", "work"]);
        assert_eq!(titles(&phone), vec!["home", "work"]);

        // Edit on the PC reaches the phone, initiated from the PC side.
        let mut d = pc.sync.read_store().unwrap();
        for b in d.boards.as_array_mut().unwrap() {
            if b["id"] == "work" {
                b["columns"][0]["cards"][0]["name"] = json!("report sent");
            }
        }
        pc.sync.write_store(&d).unwrap();
        pc.sync.run_all(true).unwrap();
        assert_eq!(card_name(&phone, "work"), "report sent");

        // Deleting a board on the phone removes it on the PC.
        let mut d = phone.sync.read_store().unwrap();
        d.boards.as_array_mut().unwrap().retain(|b| b["id"] != "home");
        phone.sync.write_store(&d).unwrap();
        phone.sync.run_all(true).unwrap();
        assert_eq!(titles(&pc), vec!["work"]);

        // A third device paired via the phone learns about the PC by gossip
        // and can sync with it directly.
        let tablet = node("Tablet", json!([]));
        let code = phone.sync.start_pairing().unwrap().code;
        tablet.sync.join(&code).unwrap();
        tablet.sync.run_all(true).unwrap();
        let known: Vec<String> = tablet.sync.status().peers.iter().map(|p| p.name.clone()).collect();
        assert!(known.contains(&"PC".to_string()), "{known:?}");
        assert_eq!(titles(&tablet), vec!["work"]);
    }

    #[test]
    fn background_image_syncs_and_is_never_erased() {
        let pc = node("PC", json!([board("work", "a")]));
        // The phone stores "no background" as null, which must not win.
        let mut phone_board = board("work", "a");
        phone_board["background"] = Value::Null;
        let phone = node("Phone", json!([phone_board]));
        let code = pc.sync.start_pairing().unwrap().code;
        phone.sync.join(&code).unwrap();
        phone.sync.run_all(true).unwrap();

        // PC picks a background picture from its disk.
        let pic_dir = std::env::temp_dir().join(format!("kanri-pic-{}", crypto::random_id(6)));
        std::fs::create_dir_all(&pic_dir).unwrap();
        let pic = pic_dir.join("holiday.png");
        std::fs::write(&pic, b"\x89PNG fake image").unwrap();
        let mut d = pc.sync.read_store().unwrap();
        d.boards[0]["background"] = json!({ "src": pic.to_string_lossy(), "blur": "4px", "brightness": "80%" });
        pc.sync.write_store(&d).unwrap();

        // Phone syncs: gets the setting and downloads the image itself.
        phone.sync.run_all(true).unwrap();
        let bg = phone.sync.read_store().unwrap().boards[0]["background"].clone();
        let src = bg["src"].as_str().unwrap().to_string();
        assert!(!src.is_empty(), "phone should have the image: {bg}");
        assert_eq!(std::fs::read(&src).unwrap(), b"\x89PNG fake image");
        assert_eq!(bg["blur"], json!("4px"));

        // Syncing back and forth keeps the PC's own background.
        pc.sync.run_all(true).unwrap();
        phone.sync.run_all(true).unwrap();
        let pc_bg = pc.sync.read_store().unwrap().boards[0]["background"].clone();
        assert!(!pc_bg["src"].as_str().unwrap_or("").is_empty(), "PC background lost: {pc_bg}");

        // Removing the background on the phone removes it everywhere.
        let mut d = phone.sync.read_store().unwrap();
        d.boards[0].as_object_mut().unwrap().remove("background");
        phone.sync.write_store(&d).unwrap();
        phone.sync.run_all(true).unwrap();
        assert!(pc.sync.read_store().unwrap().boards[0].get("background").is_none());
    }

    #[test]
    fn stranger_is_rejected_with_clear_error() {
        let a = node("A", json!([board("a", "x")]));
        let b = node("B", json!([board("b", "y")]));
        let stranger = node("Stranger", json!([board("intruder", "z")]));
        let code = a.sync.start_pairing().unwrap().code;
        b.sync.join(&code).unwrap();

        // The stranger creates its own group and learns A's address somehow.
        stranger.sync.ensure_group();
        let a_info = Sync::<MockRuntime>::device_as_peer(&a.sync.me());
        stranger.sync.upsert_peer(&a_info, None);
        let err = stranger.sync.run_all(true).unwrap_err();
        assert!(err.contains("different sync group"), "{err}");
        // B may already have synced with A in the background; what matters is
        // that nothing from the unpaired device got in.
        assert!(!titles(&a).contains(&"intruder".to_string()));
    }

    #[test]
    fn expired_or_wrong_pairing_code_fails() {
        let a = node("A", json!([]));
        let b = node("B", json!([]));
        let code = a.sync.start_pairing().unwrap().code;
        *a.sync.pairing.lock().unwrap() = None;
        let err = b.sync.join(&code).unwrap_err();
        assert!(err.contains("Pairing is not open"), "{err}");
        assert!(b.sync.join("garbage").is_err());
    }
}
