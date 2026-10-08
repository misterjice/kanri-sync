// SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Automatic discovery of group members on the local network via mDNS
//! (the same mechanism printers and Chromecasts use). Devices advertise only
//! a hashed group tag and their device id, never the group key.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

const SERVICE: &str = "_kanrisync._tcp.local.";

pub struct Found {
    pub device_id: String,
    pub addrs: Vec<String>,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: Option<String>,
}

impl Discovery {
    pub fn start<F>(device_id: &str, group_tag: &str, port: u16, on_found: F) -> Result<Self, String>
    where
        F: Fn(Found) + Send + 'static,
    {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS unavailable: {e}"))?;
        let host = format!("kanri-{}.local.", &device_id[..device_id.len().min(12)]);
        let props = [("g", group_tag), ("id", device_id)];
        let info = ServiceInfo::new(SERVICE, device_id, &host, "", port, &props[..])
            .map_err(|e| e.to_string())?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info).map_err(|e| format!("mDNS register failed: {e}"))?;

        let rx = daemon.browse(SERVICE).map_err(|e| format!("mDNS browse failed: {e}"))?;
        let own_id = device_id.to_string();
        let tag = group_tag.to_string();
        std::thread::Builder::new()
            .name("kanri-sync-mdns".into())
            .spawn(move || {
                while let Ok(ev) = rx.recv() {
                    if let ServiceEvent::ServiceResolved(info) = ev {
                        let g = info.get_property_val_str("g").unwrap_or_default();
                        let id = info.get_property_val_str("id").unwrap_or_default().to_string();
                        if g != tag || id.is_empty() || id == own_id {
                            continue;
                        }
                        let port = info.get_port();
                        let addrs = info
                            .get_addresses_v4()
                            .into_iter()
                            .map(|ip| format!("{ip}:{port}"))
                            .collect::<Vec<_>>();
                        if !addrs.is_empty() {
                            on_found(Found { device_id: id, addrs });
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { daemon, fullname: Some(fullname) })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        if let Some(f) = self.fullname.take() {
            let _ = self.daemon.unregister(&f);
        }
        let _ = self.daemon.shutdown();
    }
}
