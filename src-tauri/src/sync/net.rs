// SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! LAN transport: a small HTTP server every device runs, and the client used
//! to talk to peers. Bodies are always sealed with [`super::crypto`].

use super::crypto::{self, Key};
use super::doc::SyncDoc;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47613;
const MAX_BODY: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub port: u16,
    #[serde(default)]
    pub addrs: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub addrs: Vec<String>,
    #[serde(default)]
    pub last_seen: u64,
}

#[derive(Serialize, Deserialize)]
pub struct SyncRequest {
    pub from: DeviceInfo,
    pub peers: Vec<PeerInfo>,
    pub digest: String,
    /// Present in the second phase, when the initiator pushes its merged state.
    pub doc: Option<SyncDoc>,
}

#[derive(Serialize, Deserialize)]
pub struct SyncResponse {
    pub from: DeviceInfo,
    pub peers: Vec<PeerInfo>,
    pub digest: String,
    /// Sent only when the request carried no document and digests differ.
    pub doc: Option<SyncDoc>,
}

#[derive(Serialize, Deserialize)]
pub struct PairRequest {
    pub device: DeviceInfo,
}

#[derive(Serialize, Deserialize)]
pub struct PairResponse {
    pub group_id: String,
    pub group_key: String,
    pub from: DeviceInfo,
    pub peers: Vec<PeerInfo>,
}

/// Contents of the pairing QR code / text code.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PairCode {
    /// Candidate "ip:port" addresses of the device showing the code.
    pub a: Vec<String>,
    /// One-time token; the pairing key is derived from it.
    pub t: String,
    /// Name of the device showing the code.
    pub n: String,
}

pub const PAIR_PREFIX: &str = "kanrisync1:";

impl PairCode {
    pub fn encode(&self) -> String {
        use base64::Engine;
        let json = serde_json::to_vec(self).unwrap_or_default();
        format!("{PAIR_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
    }

    pub fn decode(s: &str) -> Result<Self, String> {
        use base64::Engine;
        let s = s.trim();
        let body = s
            .strip_prefix(PAIR_PREFIX)
            .ok_or_else(|| "This is not a Kanri Sync pairing code.".to_string())?;
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| "The pairing code is damaged or incomplete.".to_string())?;
        serde_json::from_slice(&json).map_err(|_| "The pairing code is damaged or incomplete.".to_string())
    }
}

/// IPv4 addresses of this device on local networks.
pub fn local_ipv4s() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .map(|ifs| {
            ifs.into_iter()
                .filter(|i| !i.is_loopback())
                .filter_map(|i| match i.ip() {
                    IpAddr::V4(v4) if !v4.is_link_local() => Some(v4),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    // Fallback: the address the OS would route external traffic from. No
    // packet is sent; connect() on UDP only selects a route.
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("192.0.2.1:9").is_ok() {
            if let Ok(SocketAddr::V4(a)) = sock.local_addr() {
                if !out.contains(a.ip()) && !a.ip().is_unspecified() {
                    out.insert(0, *a.ip());
                }
            }
        }
    }
    // Prefer typical home-network ranges first.
    out.sort_by_key(|ip| !ip.is_private());
    out
}

pub fn local_addrs(port: u16) -> Vec<String> {
    local_ipv4s().into_iter().map(|ip| format!("{ip}:{port}")).collect()
}

/// Outcome of contacting one address, separating "device not reachable"
/// (normal when it is switched off) from real errors.
#[derive(Debug)]
pub enum CallError {
    Unreachable(String),
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Unreachable(m) | CallError::Failed(m) => f.write_str(m),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(30))
        .build()
}

pub fn call<Req: Serialize, Resp: serde::de::DeserializeOwned>(
    addr: &str,
    path: &str,
    key: &Key,
    req: &Req,
) -> Result<Resp, CallError> {
    let body = crypto::seal(key, path, req).map_err(CallError::Failed)?;
    let url = format!("http://{addr}{path}");
    let resp = agent().post(&url).set("Content-Type", "application/octet-stream").send_bytes(&body);
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let msg = r.into_string().unwrap_or_default();
            return Err(CallError::Failed(if msg.is_empty() {
                format!("peer answered HTTP {code}")
            } else {
                msg
            }));
        }
        Err(ureq::Error::Transport(t)) => return Err(CallError::Unreachable(t.to_string())),
    };
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_BODY as u64)
        .read_to_end(&mut buf)
        .map_err(|e| CallError::Unreachable(e.to_string()))?;
    crypto::open(key, path, &buf).map_err(CallError::Failed)
}

pub struct Incoming {
    pub path: String,
    pub remote_ip: Option<IpAddr>,
    pub body: Vec<u8>,
}

/// Starts the HTTP listener, preferring the fixed port so peers can reach us
/// at a stable address. `handle` returns (status, body).
pub fn serve<F>(preferred: u16, handle: F) -> Result<u16, String>
where
    F: Fn(Incoming) -> (u16, Vec<u8>) + Send + 'static,
{
    let server = tiny_http::Server::http(("0.0.0.0", preferred))
        .or_else(|_| tiny_http::Server::http(("0.0.0.0", 0)))
        .map_err(|e| format!("could not start sync listener: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(preferred);
    std::thread::Builder::new()
        .name("kanri-sync-server".into())
        .spawn(move || {
            for mut req in server.incoming_requests() {
                let path = req.url().split('?').next().unwrap_or("").to_string();
                let remote_ip = req.remote_addr().map(|a| a.ip());
                let mut body = Vec::new();
                let read = req.as_reader().take(MAX_BODY as u64).read_to_end(&mut body);
                let (status, out) = if *req.method() != tiny_http::Method::Post {
                    (405, b"method not allowed".to_vec())
                } else if read.is_err() {
                    (400, b"could not read request".to_vec())
                } else {
                    handle(Incoming { path, remote_ip, body })
                };
                let _ = req.respond(tiny_http::Response::from_data(out).with_status_code(status));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_code_roundtrip() {
        let c = PairCode { a: vec!["192.168.1.5:47613".into()], t: "abc".into(), n: "PC".into() };
        let d = PairCode::decode(&c.encode()).unwrap();
        assert_eq!(d.a, c.a);
        assert!(PairCode::decode("hello").is_err());
    }
}
