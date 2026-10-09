// SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Board background images.
//!
//! Kanri stores a background as a path to an image file on the local disk,
//! which means nothing on another device. For sync, backgrounds are converted
//! to a *portable* form that names the image by the SHA-256 of its content:
//!
//! * local:    `{"src": "C:/…/photo.jpg", "blur": …, "brightness": …}`
//! * portable: `{"blob": "<sha256>", "ext": "jpg", "blur": …, "brightness": …}`
//!
//! Every device keeps a copy of each image in its own blob folder, so the
//! portable form can be turned back into a local path anywhere. A device that
//! has not received an image yet keeps the setting with an empty `src` (the
//! UI shows no image) until the image has been fetched from a peer.

use super::doc::StoreData;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Largest background image that is synced.
pub const MAX_BLOB: u64 = 25 * 1024 * 1024;

/// (path, size, mtime) -> (hash, ext); avoids re-hashing unchanged files.
type HashCache = HashMap<(String, u64, u128), (String, String)>;

pub struct Blobs {
    dir: PathBuf,
    cache: Mutex<HashCache>,
}

fn clean_ext(ext: &str) -> String {
    let e = ext.trim().to_ascii_lowercase();
    if !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()) {
        e
    } else {
        "img".into()
    }
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl Blobs {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir, cache: Mutex::new(HashMap::new()) }
    }

    pub fn path(&self, hash: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{hash}.{}", clean_ext(ext)))
    }

    pub fn has(&self, hash: &str, ext: &str) -> bool {
        is_hash(hash) && self.path(hash, ext).is_file()
    }

    pub fn read(&self, hash: &str, ext: &str) -> Option<Vec<u8>> {
        if !is_hash(hash) {
            return None;
        }
        std::fs::read(self.path(hash, ext)).ok()
    }

    /// Stores a received image after checking it matches its hash.
    pub fn write(&self, hash: &str, ext: &str, bytes: &[u8]) -> Result<(), String> {
        if !is_hash(hash) || sha256_hex(bytes) != hash {
            return Err("received background image is damaged".into());
        }
        let path = self.path(hash, ext);
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// If `path` is one of our blob files, returns its (hash, ext).
    fn own_blob(&self, path: &Path) -> Option<(String, String)> {
        if path.parent()? != self.dir {
            return None;
        }
        let stem = path.file_stem()?.to_str()?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("img");
        is_hash(stem).then(|| (stem.to_string(), clean_ext(ext)))
    }

    /// Copies a user-chosen image into the blob folder; returns (hash, ext).
    fn import(&self, src: &str) -> Option<(String, String)> {
        let path = Path::new(src);
        if let Some(own) = self.own_blob(path) {
            return Some(own);
        }
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_BLOB {
            return None;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let key = (src.to_string(), meta.len(), mtime);
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            if self.has(&hit.0, &hit.1) {
                return Some(hit.clone());
            }
        }
        let bytes = std::fs::read(path).ok()?;
        let hash = sha256_hex(&bytes);
        let ext = clean_ext(path.extension().and_then(|e| e.to_str()).unwrap_or("img"));
        if !self.has(&hash, &ext) {
            self.write(&hash, &ext, &bytes).ok()?;
        }
        self.cache.lock().unwrap().insert(key, (hash.clone(), ext.clone()));
        Some((hash, ext))
    }

    fn backgrounds_mut(data: &mut StoreData) -> impl Iterator<Item = &mut Map<String, Value>> {
        data.boards
            .as_array_mut()
            .into_iter()
            .flatten()
            .filter_map(|b| b.get_mut("background"))
            .filter_map(Value::as_object_mut)
    }

    /// Local store contents -> form that is synced.
    pub fn to_portable(&self, data: &mut StoreData) {
        // "No background" is stored either as null or as a missing key. Treat
        // both the same so that one device's null never overwrites another
        // device's real background.
        for b in data.boards.as_array_mut().into_iter().flatten() {
            if let Some(o) = b.as_object_mut() {
                if o.get("background").is_some_and(|v| !v.is_object()) {
                    o.remove("background");
                }
            }
        }
        for bg in Self::backgrounds_mut(data) {
            let src = bg.get("src").and_then(Value::as_str).unwrap_or("").to_string();
            let known = match (bg.get("blob").and_then(Value::as_str), bg.get("ext").and_then(Value::as_str)) {
                (Some(h), Some(e)) if is_hash(h) => Some((h.to_string(), clean_ext(e))),
                _ => None,
            };
            // The image the user currently has selected wins over a stale
            // blob reference (they may have picked a new picture).
            let resolved = if src.is_empty() { known } else { self.import(&src).or(known) };
            let Some((hash, ext)) = resolved else {
                // Unreadable local file and no blob: sync the raw setting.
                continue;
            };
            bg.remove("src");
            bg.insert("blob".into(), Value::String(hash));
            bg.insert("ext".into(), Value::String(ext));
        }
    }

    /// Synced form -> local store contents (paths into this device's blobs).
    pub fn to_local(&self, data: &mut StoreData) {
        for bg in Self::backgrounds_mut(data) {
            let (Some(hash), Some(ext)) = (
                bg.get("blob").and_then(Value::as_str).map(str::to_string),
                bg.get("ext").and_then(Value::as_str).map(str::to_string),
            ) else {
                continue;
            };
            let src = if self.has(&hash, &ext) {
                self.path(&hash, &ext).to_string_lossy().into_owned()
            } else {
                String::new()
            };
            bg.insert("src".into(), Value::String(src));
        }
    }

    /// Images referenced by `boards` (portable or local form) that this device lacks.
    pub fn missing(&self, boards: &[Value]) -> BTreeSet<(String, String)> {
        boards
            .iter()
            .filter_map(|b| b.get("background"))
            .filter_map(|bg| {
                let h = bg.get("blob")?.as_str()?;
                let e = bg.get("ext")?.as_str()?;
                (is_hash(h) && !self.has(h, e)).then(|| (h.to_string(), clean_ext(e)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kanri-blobs-{name}-{}", super::super::crypto::random_id(6)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn data(bg: Value) -> StoreData {
        StoreData { boards: json!([{ "id": "b1", "title": "B", "columns": [], "background": bg }]), pins: json!([]) }
    }

    #[test]
    fn background_travels_between_devices() {
        let pc_files = tmpdir("pcfiles");
        let photo = pc_files.join("Photo.JPG");
        std::fs::write(&photo, b"fake image bytes").unwrap();
        let pc = Blobs::new(tmpdir("pc"));
        let phone = Blobs::new(tmpdir("phone"));

        // PC: local path -> portable reference (image copied into its blobs).
        let mut d = data(json!({ "src": photo.to_string_lossy(), "blur": "8px", "brightness": "90%" }));
        pc.to_portable(&mut d);
        let bg = d.boards[0]["background"].clone();
        let hash = bg["blob"].as_str().unwrap().to_string();
        assert_eq!(bg["ext"], json!("jpg"));
        assert!(bg.get("src").is_none());

        // Phone before the image arrives: setting kept, no image shown.
        let mut on_phone = d.clone();
        phone.to_local(&mut on_phone);
        assert_eq!(on_phone.boards[0]["background"]["src"], json!(""));
        assert_eq!(phone.missing(on_phone.boards.as_array().unwrap()).len(), 1);
        // ...and converting back is stable, so nothing is re-synced.
        let mut back = on_phone.clone();
        phone.to_portable(&mut back);
        assert_eq!(back, d);

        // Image fetched from the PC: phone now points at its own copy.
        let bytes = pc.read(&hash, "jpg").unwrap();
        assert!(phone.write(&hash, "jpg", b"tampered").is_err());
        phone.write(&hash, "jpg", &bytes).unwrap();
        let mut on_phone = d.clone();
        phone.to_local(&mut on_phone);
        let src = on_phone.boards[0]["background"]["src"].as_str().unwrap().to_string();
        assert!(src.ends_with(&format!("{hash}.jpg")));
        assert_eq!(on_phone.boards[0]["background"]["blur"], json!("8px"));
        let mut back = on_phone.clone();
        phone.to_portable(&mut back);
        assert_eq!(back, d);
    }

    #[test]
    fn null_background_is_same_as_none() {
        let blobs = Blobs::new(tmpdir("nul"));
        let mut d = data(Value::Null);
        blobs.to_portable(&mut d);
        assert!(d.boards[0].get("background").is_none());
    }

    #[test]
    fn new_picture_replaces_old_reference() {
        let files = tmpdir("files");
        let a = files.join("a.png");
        let b = files.join("b.png");
        std::fs::write(&a, b"aaaa").unwrap();
        std::fs::write(&b, b"bbbb").unwrap();
        let blobs = Blobs::new(tmpdir("dev"));
        let mut d = data(json!({ "src": a.to_string_lossy(), "blur": "0px", "brightness": "100%" }));
        blobs.to_portable(&mut d);
        let first = d.boards[0]["background"]["blob"].clone();
        blobs.to_local(&mut d);
        // User picks another picture: UI writes a plain {src, blur, brightness}.
        d.boards[0]["background"]["src"] = json!(b.to_string_lossy());
        blobs.to_portable(&mut d);
        assert_ne!(d.boards[0]["background"]["blob"], first);
        assert_eq!(d.boards[0]["background"]["blob"], json!(sha256_hex(b"bbbb")));
    }
}
