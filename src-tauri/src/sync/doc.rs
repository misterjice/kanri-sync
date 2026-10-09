// SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replicated document model used for LAN sync.
//!
//! The nested `boards` array that Kanri stores is flattened into independent
//! entities (boards, columns, cards, plus the pin list and board order). Each
//! entity carries last-writer-wins registers for its content, its parent and
//! its child ordering, so concurrent edits on different devices merge
//! field-by-field instead of overwriting whole files:
//!
//! * editing a card on the phone while moving it on the PC keeps both changes;
//! * adding cards to the same column on two devices keeps both cards;
//! * deletions are kept as tombstones so deleted items do not reappear.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

/// Logical timestamp: (hybrid logical clock millis, device id). Ordered
/// lexicographically, so ties on time are broken deterministically by device.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub t: u64,
    pub d: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reg<T> {
    pub v: T,
    pub s: Stamp,
}

impl<T: Clone> Reg<T> {
    fn merge(&mut self, other: &Reg<T>) -> bool {
        if other.s > self.s {
            *self = other.clone();
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Board,
    Column,
    Card,
    /// Singletons: the pin list and the board order.
    Meta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub kind: Kind,
    pub body: Reg<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Reg<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Reg<Vec<String>>>,
    /// Deletion register; the entity is deleted when this is the newest write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub del: Option<Stamp>,
}

impl Entity {
    fn newest_write(&self) -> &Stamp {
        let mut s = &self.body.s;
        if let Some(p) = &self.parent {
            if p.s > *s {
                s = &p.s;
            }
        }
        if let Some(o) = &self.order {
            if o.s > *s {
                s = &o.s;
            }
        }
        s
    }

    pub fn is_deleted(&self) -> bool {
        match &self.del {
            Some(d) => d > self.newest_write(),
            None => false,
        }
    }

    fn merge(&mut self, other: &Entity) -> bool {
        let mut changed = self.body.merge(&other.body);
        changed |= merge_opt(&mut self.parent, &other.parent);
        changed |= merge_opt(&mut self.order, &other.order);
        if let Some(od) = &other.del {
            if self.del.as_ref().is_none_or(|d| od > d) {
                self.del = Some(od.clone());
                changed = true;
            }
        }
        changed
    }
}

fn merge_opt<T: Clone>(a: &mut Option<Reg<T>>, b: &Option<Reg<T>>) -> bool {
    match (a.as_mut(), b) {
        (Some(x), Some(y)) => x.merge(y),
        (None, Some(y)) => {
            *a = Some(y.clone());
            true
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncDoc {
    pub entities: BTreeMap<String, Entity>,
}

const PINS: &str = "m:pins";
const BOARD_ORDER: &str = "m:boards";

fn key(kind: Kind, id: &str) -> String {
    match kind {
        Kind::Board => format!("b:{id}"),
        Kind::Column => format!("l:{id}"),
        Kind::Card => format!("c:{id}"),
        Kind::Meta => id.to_string(),
    }
}

fn id_of(key: &str) -> &str {
    key.split_once(':').map_or(key, |(_, id)| id)
}

/// A flattened view of the store contents before stamping.
struct Flat {
    kind: Kind,
    body: Value,
    parent: Option<String>,
    order: Option<Vec<String>>,
}

/// Kanri's on-disk data that takes part in sync.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoreData {
    pub boards: Value,
    pub pins: Value,
}

/// Ensures every board, column and card has a string id that is unique
/// among its kind. Returns true when ids were changed and the caller must
/// write the data back to the store.
///
/// Duplicates happen in practice: Kanri's "Duplicate board" copies the
/// column and card ids. Sync keys entities by id, so without this the copy
/// and the original would fight over the same columns and cards. The first
/// occurrence keeps its id; later ones get fresh ids.
pub fn normalize_ids(data: &mut StoreData, mut gen: impl FnMut() -> String) -> bool {
    let mut changed = false;
    let mut seen: [HashSet<String>; 3] = Default::default();
    let mut ensure = |obj: &mut Map<String, Value>, kind: usize, changed: &mut bool| {
        let current = match obj.get("id") {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        if let Some(id) = current {
            if seen[kind].insert(id) {
                return;
            }
        }
        let mut id = gen();
        while !seen[kind].insert(id.clone()) {
            id = gen();
        }
        obj.insert("id".into(), Value::String(id));
        *changed = true;
    };
    if let Value::Array(boards) = &mut data.boards {
        for b in boards.iter_mut() {
            let Some(b) = b.as_object_mut() else { continue };
            ensure(b, 0, &mut changed);
            let Some(Value::Array(cols)) = b.get_mut("columns") else { continue };
            for c in cols.iter_mut() {
                let Some(c) = c.as_object_mut() else { continue };
                ensure(c, 1, &mut changed);
                let Some(Value::Array(cards)) = c.get_mut("cards") else { continue };
                for k in cards.iter_mut() {
                    if let Some(k) = k.as_object_mut() {
                        ensure(k, 2, &mut changed);
                    }
                }
            }
        }
    }
    changed
}

fn str_id(v: &Map<String, Value>) -> Option<String> {
    v.get("id").and_then(Value::as_str).map(str::to_string)
}

fn flatten(data: &StoreData) -> BTreeMap<String, Flat> {
    let mut out = BTreeMap::new();
    let mut board_order = Vec::new();
    if let Value::Array(boards) = &data.boards {
        for b in boards {
            let Some(bo) = b.as_object() else { continue };
            let Some(bid) = str_id(bo) else { continue };
            let mut body = bo.clone();
            let cols = body.remove("columns");
            // lastEdited changes on every card edit; it is derived on rebuild.
            body.remove("lastEdited");
            // Backgrounds arrive here in portable form (see blobs.rs).
            let mut col_order = Vec::new();
            if let Some(Value::Array(cols)) = cols {
                for c in cols {
                    let Some(co) = c.as_object() else { continue };
                    let Some(cid) = str_id(co) else { continue };
                    let mut cbody = co.clone();
                    let cards = cbody.remove("cards");
                    let mut card_order = Vec::new();
                    if let Some(Value::Array(cards)) = cards {
                        for k in cards {
                            let Some(ko) = k.as_object() else { continue };
                            let Some(kid) = str_id(ko) else { continue };
                            card_order.push(kid.clone());
                            out.insert(
                                key(Kind::Card, &kid),
                                Flat { kind: Kind::Card, body: k.clone(), parent: Some(cid.clone()), order: None },
                            );
                        }
                    }
                    col_order.push(cid.clone());
                    out.insert(
                        key(Kind::Column, &cid),
                        Flat {
                            kind: Kind::Column,
                            body: Value::Object(cbody),
                            parent: Some(bid.clone()),
                            order: Some(card_order),
                        },
                    );
                }
            }
            board_order.push(bid.clone());
            out.insert(
                key(Kind::Board, &bid),
                Flat { kind: Kind::Board, body: Value::Object(body), parent: None, order: Some(col_order) },
            );
        }
    }
    out.insert(
        BOARD_ORDER.into(),
        Flat { kind: Kind::Meta, body: Value::Null, parent: None, order: Some(board_order) },
    );
    let pins = if data.pins.is_array() { data.pins.clone() } else { Value::Array(vec![]) };
    out.insert(PINS.into(), Flat { kind: Kind::Meta, body: pins, parent: None, order: None });
    out
}

/// Hybrid logical clock: monotonic, never behind any timestamp it has seen.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Clock {
    pub last: u64,
    pub device: String,
}

impl Clock {
    pub fn tick(&mut self, now_ms: u64) -> Stamp {
        self.last = now_ms.max(self.last + 1);
        Stamp { t: self.last, d: self.device.clone() }
    }

    pub fn observe(&mut self, doc: &SyncDoc) {
        for e in doc.entities.values() {
            let mut t = e.newest_write().t;
            if let Some(d) = &e.del {
                t = t.max(d.t);
            }
            self.last = self.last.max(t);
        }
    }
}

impl SyncDoc {
    /// Records local edits: every register whose value differs from the
    /// current store contents gets a fresh stamp. Returns true if anything changed.
    pub fn stamp(&mut self, data: &StoreData, clock: &mut Clock, now_ms: u64) -> bool {
        let flat = flatten(data);
        let mut changed = false;
        for (k, f) in &flat {
            match self.entities.get_mut(k) {
                Some(e) if !e.is_deleted() => {
                    if e.body.v != f.body {
                        e.body = Reg { v: f.body.clone(), s: clock.tick(now_ms) };
                        changed = true;
                    }
                    if let Some(p) = &f.parent {
                        if e.parent.as_ref().map(|r| &r.v) != Some(p) {
                            e.parent = Some(Reg { v: p.clone(), s: clock.tick(now_ms) });
                            changed = true;
                        }
                    }
                    if let Some(o) = &f.order {
                        // Only the relative order of children this device can
                        // see matters; compare against the rendered order.
                        if e.order.as_ref().map(|r| &r.v) != Some(o) {
                            e.order = Some(Reg { v: o.clone(), s: clock.tick(now_ms) });
                            changed = true;
                        }
                    }
                }
                _ => {
                    let s = clock.tick(now_ms);
                    let prev_del = self.entities.get(k).and_then(|e| e.del.clone());
                    self.entities.insert(
                        k.clone(),
                        Entity {
                            kind: f.kind,
                            body: Reg { v: f.body.clone(), s: s.clone() },
                            parent: f.parent.clone().map(|v| Reg { v, s: s.clone() }),
                            order: f.order.clone().map(|v| Reg { v, s: s.clone() }),
                            del: prev_del,
                        },
                    );
                    changed = true;
                }
            }
        }
        let gone: Vec<String> = self
            .entities
            .iter()
            .filter(|(k, e)| !e.is_deleted() && !flat.contains_key(*k))
            .map(|(k, _)| k.clone())
            .collect();
        for k in gone {
            let s = clock.tick(now_ms);
            if let Some(e) = self.entities.get_mut(&k) {
                e.del = Some(s);
                changed = true;
            }
        }
        changed
    }

    /// Merges a remote document into this one. Commutative, associative and
    /// idempotent, so devices converge regardless of sync order.
    pub fn merge(&mut self, other: &SyncDoc) -> bool {
        let mut changed = false;
        for (k, oe) in &other.entities {
            match self.entities.get_mut(k) {
                Some(e) => changed |= e.merge(oe),
                None => {
                    self.entities.insert(k.clone(), oe.clone());
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(bytes))
    }

    fn live(&self, kind: Kind) -> impl Iterator<Item = (&String, &Entity)> {
        self.entities.iter().filter(move |(_, e)| e.kind == kind && !e.is_deleted())
    }

    /// Children of `parent` in the order the parent records, followed by any
    /// children the parent's order does not know about yet (concurrent adds).
    fn ordered_children(&self, kind: Kind, parent_id: &str, order: Option<&Vec<String>>) -> Vec<&Entity> {
        let mut kids: Vec<(&String, &Entity)> = self
            .live(kind)
            .filter(|(_, e)| e.parent.as_ref().is_some_and(|p| p.v == parent_id))
            .collect();
        let pos: BTreeMap<&str, usize> = order
            .map(|o| o.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect())
            .unwrap_or_default();
        kids.sort_by(|(ka, a), (kb, b)| {
            let pa = pos.get(id_of(ka)).copied().unwrap_or(usize::MAX);
            let pb = pos.get(id_of(kb)).copied().unwrap_or(usize::MAX);
            pa.cmp(&pb).then_with(|| a.body.s.cmp(&b.body.s))
        });
        kids.into_iter().map(|(_, e)| e).collect()
    }

    /// Rebuilds Kanri's nested store layout from the merged entities.
    pub fn render(&self, previous: &StoreData) -> StoreData {
        // Keep the device-local lastEdited from the local copy.
        let mut last_edited: BTreeMap<String, Value> = BTreeMap::new();
        if let Value::Array(bs) = &previous.boards {
            for b in bs {
                let Some(id) = b.get("id").and_then(Value::as_str) else { continue };
                if let Some(le) = b.get("lastEdited") {
                    last_edited.insert(id.to_string(), le.clone());
                }
            }
        }
        let empty_order = Vec::new();
        let board_order = self
            .entities
            .get(BOARD_ORDER)
            .and_then(|e| e.order.as_ref())
            .map(|o| &o.v)
            .unwrap_or(&empty_order);
        let mut boards: Vec<(&String, &Entity)> = self.live(Kind::Board).collect();
        let pos: BTreeMap<&str, usize> = board_order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        boards.sort_by(|(ka, a), (kb, b)| {
            let pa = pos.get(id_of(ka)).copied().unwrap_or(usize::MAX);
            let pb = pos.get(id_of(kb)).copied().unwrap_or(usize::MAX);
            pa.cmp(&pb).then_with(|| a.body.s.cmp(&b.body.s))
        });

        let mut out = Vec::new();
        for (bk, be) in boards {
            let bid = id_of(bk);
            let mut board = be.body.v.as_object().cloned().unwrap_or_default();
            let mut cols = Vec::new();
            for ce in self.ordered_children(Kind::Column, bid, be.order.as_ref().map(|o| &o.v)) {
                let mut col = ce.body.v.as_object().cloned().unwrap_or_default();
                let cid = col.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                let cards: Vec<Value> = self
                    .ordered_children(Kind::Card, &cid, ce.order.as_ref().map(|o| &o.v))
                    .into_iter()
                    .map(|k| k.body.v.clone())
                    .collect();
                col.insert("cards".into(), Value::Array(cards));
                cols.push(Value::Object(col));
            }
            board.insert("columns".into(), Value::Array(cols));
            if let Some(le) = last_edited.get(bid) {
                board.insert("lastEdited".into(), le.clone());
            }

            out.push(Value::Object(board));
        }

        let pins = self
            .entities
            .get(PINS)
            .filter(|e| !e.is_deleted())
            .map(|e| e.body.v.clone())
            .unwrap_or(Value::Array(vec![]));
        // Drop pins that point at boards which no longer exist.
        let board_ids: HashSet<&str> = out.iter().filter_map(|b| b.get("id").and_then(Value::as_str)).collect();
        let pins = match pins {
            Value::Array(ps) => Value::Array(
                ps.into_iter()
                    .filter(|p| p.get("id").and_then(Value::as_str).is_some_and(|id| board_ids.contains(id)))
                    .collect(),
            ),
            other => other,
        };

        StoreData { boards: Value::Array(out), pins }
    }

    /// Boards whose rendered content differs between two renders; used to
    /// bump lastEdited on boards that received remote changes.
    pub fn touch_changed_boards(before: &StoreData, after: &mut StoreData, now_iso: &str) {
        let mut old: BTreeMap<String, Value> = BTreeMap::new();
        if let Value::Array(bs) = &before.boards {
            for b in bs {
                if let Some(id) = b.get("id").and_then(Value::as_str) {
                    let mut b = b.clone();
                    if let Some(o) = b.as_object_mut() {
                        o.remove("lastEdited");
                    }
                    old.insert(id.to_string(), b);
                }
            }
        }
        if let Value::Array(bs) = &mut after.boards {
            for b in bs.iter_mut() {
                let Some(o) = b.as_object_mut() else { continue };
                let id = o.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                let mut cmp = o.clone();
                cmp.remove("lastEdited");
                if old.get(&id) != Some(&Value::Object(cmp)) {
                    o.insert("lastEdited".into(), Value::String(now_iso.to_string()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data(boards: Value) -> StoreData {
        StoreData { boards, pins: json!([]) }
    }

    fn board(cols: Value) -> Value {
        json!([{ "id": "b1", "title": "Board", "columns": cols }])
    }

    fn device(name: &str) -> (SyncDoc, Clock) {
        (SyncDoc::default(), Clock { last: 0, device: name.into() })
    }

    fn cards_of(d: &StoreData, col: usize) -> Vec<String> {
        d.boards[0]["columns"][col]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect()
    }

    fn base() -> StoreData {
        data(board(json!([
            { "id": "l1", "title": "Todo", "cards": [
                { "id": "c1", "name": "A" }, { "id": "c2", "name": "B" }
            ]},
            { "id": "l2", "title": "Done", "cards": [] }
        ])))
    }

    /// Two devices start from the same synced state.
    fn pair() -> (SyncDoc, Clock, SyncDoc, Clock) {
        let (mut a, mut ca) = device("A");
        a.stamp(&base(), &mut ca, 1000);
        let (mut b, mut cb) = device("B");
        b.merge(&a);
        cb.observe(&b);
        (a, ca, b, cb)
    }

    #[test]
    fn roundtrip_render_matches_store() {
        let (a, _, _, _) = pair();
        assert_eq!(a.render(&base()), base());
    }

    #[test]
    fn concurrent_adds_to_same_column_keep_both() {
        let (mut a, mut ca, mut b, mut cb) = pair();
        let mut da = base();
        da.boards[0]["columns"][0]["cards"].as_array_mut().unwrap().push(json!({"id": "c3", "name": "from A"}));
        a.stamp(&da, &mut ca, 2000);
        let mut db = base();
        db.boards[0]["columns"][0]["cards"].as_array_mut().unwrap().push(json!({"id": "c4", "name": "from B"}));
        b.stamp(&db, &mut cb, 2001);

        a.merge(&b);
        b.merge(&a);
        assert_eq!(a, b);
        let names = cards_of(&a.render(&base()), 0);
        assert!(names.contains(&"from A".to_string()) && names.contains(&"from B".to_string()));
        assert_eq!(&names[..2], &["A", "B"]);
    }

    #[test]
    fn edit_and_move_of_same_card_both_survive() {
        let (mut a, mut ca, mut b, mut cb) = pair();
        // A renames c1.
        let mut da = base();
        da.boards[0]["columns"][0]["cards"][0]["name"] = json!("A renamed");
        a.stamp(&da, &mut ca, 2000);
        // B moves c1 to Done.
        let db = data(board(json!([
            { "id": "l1", "title": "Todo", "cards": [{ "id": "c2", "name": "B" }]},
            { "id": "l2", "title": "Done", "cards": [{ "id": "c1", "name": "A" }]}
        ])));
        b.stamp(&db, &mut cb, 2001);

        a.merge(&b);
        let r = a.render(&base());
        assert_eq!(cards_of(&r, 0), vec!["B"]);
        assert_eq!(cards_of(&r, 1), vec!["A renamed"]);
    }

    #[test]
    fn deletion_propagates_and_does_not_resurrect() {
        let (mut a, mut ca, mut b, cb) = pair();
        let da = data(board(json!([
            { "id": "l1", "title": "Todo", "cards": [{ "id": "c2", "name": "B" }]},
            { "id": "l2", "title": "Done", "cards": [] }
        ])));
        a.stamp(&da, &mut ca, 2000);
        b.merge(&a);
        // B re-stamps its (now stale) store copy only after rendering the merge.
        let rb = b.render(&base());
        assert_eq!(cards_of(&rb, 0), vec!["B"]);
        let _ = cb;
        // Merging again in either direction is a no-op.
        assert!(!a.merge(&b));
        assert!(!b.merge(&a));
    }

    #[test]
    fn edit_after_delete_wins() {
        let (mut a, mut ca, mut b, mut cb) = pair();
        let da = data(board(json!([
            { "id": "l1", "title": "Todo", "cards": [{ "id": "c2", "name": "B" }]},
            { "id": "l2", "title": "Done", "cards": [] }
        ])));
        a.stamp(&da, &mut ca, 2000);
        let mut db = base();
        db.boards[0]["columns"][0]["cards"][0]["name"] = json!("A edited later");
        b.stamp(&db, &mut cb, 3000);
        a.merge(&b);
        // The edit is newer than the delete, so the card comes back (at the
        // end, since the deleting device's column order no longer lists it).
        assert_eq!(cards_of(&a.render(&base()), 0), vec!["B", "A edited later"]);
    }

    #[test]
    fn merge_is_commutative() {
        let (mut a, mut ca, mut b, mut cb) = pair();
        let mut da = base();
        da.boards[0]["title"] = json!("Title A");
        a.stamp(&da, &mut ca, 2000);
        let mut db = base();
        db.boards[0]["title"] = json!("Title B");
        b.stamp(&db, &mut cb, 2000);
        let mut ab = a.clone();
        ab.merge(&b);
        let mut ba = b.clone();
        ba.merge(&a);
        assert_eq!(ab, ba);
        assert_eq!(ab.digest(), ba.digest());
        // Same millisecond: device id breaks the tie deterministically.
        assert_eq!(ab.render(&base()).boards[0]["title"], json!("Title B"));
    }

    #[test]
    fn stamping_rendered_output_is_noop() {
        let (mut a, mut ca, b, _) = pair();
        let mut x = a.clone();
        x.merge(&b);
        let r = x.render(&base());
        assert!(!a.stamp(&r, &mut ca, 5000));
    }

    #[test]
    fn last_edited_is_ignored_for_change_detection() {
        let (mut a, mut ca, _, _) = pair();
        let mut d = base();
        d.boards[0]["lastEdited"] = json!("2026-01-01T00:00:00.000Z");
        assert!(!a.stamp(&d, &mut ca, 5000));
    }

    #[test]
    fn normalize_makes_duplicated_ids_unique() {
        let board = json!({ "id": "b1", "title": "B", "columns": [
            { "id": "c1", "title": "C", "cards": [{ "id": "k1", "name": "x" }] } ] });
        let mut copy = board.clone();
        copy["id"] = json!("b2");
        let mut d = StoreData { boards: json!([board, copy]), pins: json!([]) };
        let mut n = 0;
        assert!(normalize_ids(&mut d, || { n += 1; format!("new{n}") }));
        assert_eq!(d.boards[0]["columns"][0]["id"], json!("c1"));
        assert_eq!(d.boards[0]["columns"][0]["cards"][0]["id"], json!("k1"));
        assert_ne!(d.boards[1]["columns"][0]["id"], json!("c1"));
        assert_ne!(d.boards[1]["columns"][0]["cards"][0]["id"], json!("k1"));
        // Both boards keep their own column and card after a sync round trip.
        let mut doc = SyncDoc::default();
        let mut clock = Clock::default();
        doc.stamp(&d, &mut clock, 1);
        let out = doc.render(&StoreData::default());
        assert_eq!(out.boards[0]["columns"][0]["cards"].as_array().unwrap().len(), 1);
        assert_eq!(out.boards[1]["columns"][0]["cards"].as_array().unwrap().len(), 1);
        assert!(!normalize_ids(&mut d, || unreachable!()));
    }

    #[test]
    fn normalize_assigns_missing_ids() {
        let mut d = data(json!([{ "title": "x", "columns": [{ "title": "c", "cards": [{ "name": "k" }] }] }]));
        let mut n = 0;
        assert!(normalize_ids(&mut d, || { n += 1; format!("id{n}") }));
        assert_eq!(d.boards[0]["columns"][0]["cards"][0]["id"], json!("id3"));
        assert!(!normalize_ids(&mut d, || unreachable!()));
    }

    #[test]
    fn pins_for_deleted_boards_are_dropped() {
        let (mut a, mut ca) = device("A");
        let mut d = base();
        d.pins = json!([{ "id": "b1", "title": "Board" }, { "id": "gone", "title": "x" }]);
        a.stamp(&d, &mut ca, 1000);
        let r = a.render(&d);
        assert_eq!(r.pins, json!([{ "id": "b1", "title": "Board" }]));
    }
}
