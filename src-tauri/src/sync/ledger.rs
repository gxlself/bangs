//! What this device knows about every synced record, and what it still owes
//! the cloud. No Tauri, no CloudKit: the rules of docs/sync.md and nothing
//! else, so they can be tested on their own — against the same vectors
//! (docs/sync-vectors.json) the Swift side runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const TODO: &str = "todo";
pub const SESSION: &str = "session";
pub const CLIP: &str = "clip";
pub const SHELF: &str = "shelf";

/// One synced record, in the wire format of docs/sync.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub kind: String,
    pub id: String,
    /// Unix milliseconds.
    pub updated_at: u64,
    /// The device that wrote this version.
    pub device: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub body: Value,
    /// A local file: the one to upload, or the copy already downloaded.
    #[serde(default)]
    pub asset: Option<String>,
}

impl Record {
    /// `kind:id`, which is also the CloudKit recordName.
    pub fn key(&self) -> String {
        key_of(&self.kind, &self.id)
    }
}

pub fn key_of(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// What is kept about a key: enough to tell whether another version is newer,
/// and whether a local change is a change at all. Tombstones stay, so a late
/// old version cannot bring a deleted record back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    updated_at: u64,
    device: String,
    deleted: bool,
    /// Of the body, so an unchanged mirror is not pushed again.
    hash: u64,
}

impl Version {
    fn of(record: &Record) -> Self {
        Self {
            updated_at: record.updated_at,
            device: record.device.clone(),
            deleted: record.deleted,
            hash: if record.deleted { 0 } else { hash_body(&record.body) },
        }
    }
}

/// Last writer wins: the later time, and on a tie the larger device id.
/// The very same version is not a win — that is our own record coming back.
pub fn wins(a: (u64, &str), b: (u64, &str)) -> bool {
    a > b
}

/// The version time of a new local change: the clock, but never at or below
/// what this key already has, so a slow clock cannot lose to its own past.
pub fn next_stamp(prev: Option<u64>, now: u64) -> u64 {
    match prev {
        Some(prev) => now.max(prev + 1),
        None => now,
    }
}

/// What was sent, to tell an answer about it from one about a newer change.
#[derive(Debug, Clone)]
struct Sent {
    updated_at: u64,
    device: String,
}

/// A record that should exist, for `Ledger::reconcile`.
pub struct Desired {
    pub id: String,
    pub body: Value,
    pub asset: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub device: String,
    versions: BTreeMap<String, Version>,
    /// Changes the cloud has not acknowledged yet.
    outbox: BTreeMap<String, Record>,
    /// Sent and waiting for an answer. Not saved: after a restart whatever is
    /// still in the outbox is simply sent again.
    #[serde(skip)]
    in_flight: HashMap<String, Sent>,
}

impl Ledger {
    pub fn new(device: String) -> Self {
        Self { device, ..Self::default() }
    }

    /// Reads a saved ledger. A file that cannot be parsed yields `None`; the
    /// caller decides whether to start over.
    pub fn load(path: &Path) -> Option<Self> {
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Writes next to the file first and renames, so a crash never leaves half
    /// a ledger behind.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        // Compact: it is rewritten on every change and nobody reads it by hand.
        fs::write(&temp, serde_json::to_vec(self).unwrap_or_default())?;
        fs::rename(temp, path)
    }

    pub fn has_version(&self, kind: &str, id: &str) -> bool {
        self.versions.contains_key(&key_of(kind, id))
    }

    /// Changes that still have to reach the cloud.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.outbox.len()
    }

    /// Takes in what came from the cloud, in order. Returns the records that
    /// beat what this device had — the ones to apply to the local data.
    pub fn apply_remote(&mut self, incoming: Vec<Record>) -> Vec<Record> {
        let mut adopted = Vec::new();
        for record in incoming {
            let key = record.key();
            let wins = match self.versions.get(&key) {
                None => true,
                Some(known) => wins((record.updated_at, &record.device), (known.updated_at, &known.device)),
            };
            if !wins {
                continue;
            }
            self.versions.insert(key.clone(), Version::of(&record));
            // Whatever was waiting to be sent for this key is older now.
            self.outbox.remove(&key);
            adopted.push(record);
        }
        adopted
    }

    /// A record was added or edited here. `None` when it is the same as the
    /// version already known.
    pub fn local_upsert(
        &mut self,
        kind: &str,
        id: &str,
        body: Value,
        asset: Option<String>,
        now: u64,
    ) -> Option<Record> {
        let key = key_of(kind, id);
        let hash = hash_body(&body);
        let previous = self.versions.get(&key);
        if previous.is_some_and(|known| !known.deleted && known.hash == hash) {
            return None;
        }
        let record = Record {
            kind: kind.to_string(),
            id: id.to_string(),
            updated_at: next_stamp(previous.map(|known| known.updated_at), now),
            device: self.device.clone(),
            deleted: false,
            body,
            asset,
        };
        self.commit(record)
    }

    /// A record went away here. `None` when it was never known or is already
    /// a tombstone.
    pub fn local_delete(&mut self, kind: &str, id: &str, now: u64) -> Option<Record> {
        let key = key_of(kind, id);
        let previous = self.versions.get(&key)?;
        if previous.deleted {
            return None;
        }
        let record = Record {
            kind: kind.to_string(),
            id: id.to_string(),
            updated_at: next_stamp(Some(previous.updated_at), now),
            device: self.device.clone(),
            deleted: true,
            body: json!({}),
            asset: None,
        };
        self.commit(record)
    }

    fn commit(&mut self, record: Record) -> Option<Record> {
        let key = record.key();
        self.versions.insert(key.clone(), Version::of(&record));
        self.outbox.insert(key, record.clone());
        Some(record)
    }

    /// Makes the cloud's records of this `kind` that this device wrote match
    /// `desired`: new and changed ones are upserted, vanished ones are
    /// deleted. Records written by other devices are left alone.
    pub fn reconcile(&mut self, kind: &str, desired: Vec<Desired>, now: u64) -> Vec<Record> {
        let mut changed = Vec::new();
        let mut keep = HashSet::new();
        for item in desired {
            keep.insert(item.id.clone());
            if let Some(record) = self.local_upsert(kind, &item.id, item.body, item.asset, now) {
                changed.push(record);
            }
        }
        let prefix = format!("{kind}:");
        let gone: Vec<String> = self
            .versions
            .iter()
            .filter(|(key, known)| key.starts_with(&prefix) && !known.deleted && known.device == self.device)
            .map(|(key, _)| key[prefix.len()..].to_string())
            .filter(|id| !keep.contains(id))
            .collect();
        for id in gone {
            if let Some(record) = self.local_delete(kind, &id, now) {
                changed.push(record);
            }
        }
        changed
    }

    /// Hands out what is waiting to be sent and marks it as in flight, so the
    /// next call does not send it again while the library is still on it. The
    /// library answers every record it is given; if it is restarted instead,
    /// `release_all` puts everything back.
    pub fn take_outbox(&mut self) -> Vec<Record> {
        let mut batch = Vec::new();
        for (key, record) in &self.outbox {
            if !self.in_flight.contains_key(key) {
                batch.push(record.clone());
            }
        }
        for record in &batch {
            self.in_flight.insert(record.key(), Sent { updated_at: record.updated_at, device: record.device.clone() });
        }
        batch
    }

    /// The library started over and will not answer for what it had: all of
    /// it goes out again.
    pub fn release_all(&mut self) {
        self.in_flight.clear();
    }

    /// The cloud took these (or decided against them): they are no longer owed.
    /// A key that changed again while in flight keeps its newer change.
    pub fn acknowledge(&mut self, keys: &[String]) {
        for key in keys {
            let sent = self.in_flight.remove(key);
            let same = match (self.outbox.get(key), &sent) {
                (Some(owed), Some(sent)) => owed.updated_at == sent.updated_at && owed.device == sent.device,
                _ => false,
            };
            if same {
                self.outbox.remove(key);
            }
        }
    }

    /// The push failed: keep these owed so they go out again.
    pub fn release(&mut self, keys: &[String]) {
        for key in keys {
            self.in_flight.remove(key);
        }
    }

    /// Forgets tombstones that nothing is waiting on once they are older than
    /// `ttl_for(kind)`.
    pub fn purge(&mut self, now: u64, ttl_for: impl Fn(&str) -> u64) {
        let outbox = &self.outbox;
        self.versions.retain(|key, known| {
            let kind = key.split(':').next().unwrap_or("");
            !(known.deleted && known.updated_at.saturating_add(ttl_for(kind)) < now && !outbox.contains_key(key))
        });
    }

    /// The ids of the live records of `kind`.
    pub fn live_ids(&self, kind: &str) -> Vec<String> {
        self.ids(kind, false)
    }

    /// The ids of `kind` this device knows were deleted.
    pub fn deleted_ids(&self, kind: &str) -> Vec<String> {
        self.ids(kind, true)
    }

    fn ids(&self, kind: &str, deleted: bool) -> Vec<String> {
        let prefix = format!("{kind}:");
        self.versions
            .iter()
            .filter(|(key, known)| key.starts_with(&prefix) && known.deleted == deleted)
            .map(|(key, _)| key[prefix.len()..].to_string())
            .collect()
    }

    #[cfg(test)]
    fn stamp_of(&self, key: &str) -> Option<(u64, String, bool)> {
        self.versions.get(key).map(|known| (known.updated_at, known.device.clone(), known.deleted))
    }

    #[cfg(test)]
    fn seed(&mut self, key: &str, updated_at: u64, device: &str, deleted: bool) {
        self.versions.insert(
            key.to_string(),
            Version { updated_at, device: device.to_string(), deleted, hash: 0 },
        );
    }
}

/// JSON with the keys in order, so the same body always hashes the same
/// whatever order its keys were built in.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn hash_body(body: &Value) -> u64 {
    let mut text = String::new();
    canonical(body, &mut text);
    fnv1a(text.as_bytes())
}

/// FNV-1a, 64 bit: stable across runs and Rust versions, which a record id
/// needs and `DefaultHasher` does not promise.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn fnv_hex(text: &str) -> String {
    format!("{:016x}", fnv1a(text.as_bytes()))
}

/// A record id has to be ASCII `[A-Za-z0-9._-]{1,128}` (a CloudKit recordName
/// restriction); anything else is replaced by its hash.
pub fn clean_id(id: &str) -> String {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if ok {
        id.to_string()
    } else {
        fnv_hex(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: &str = include_str!("../../../docs/sync-vectors.json");

    fn vectors() -> Value {
        serde_json::from_str(VECTORS).expect("docs/sync-vectors.json is valid JSON")
    }

    fn version_of(side: &Value) -> (u64, String) {
        (side["updatedAt"].as_u64().unwrap(), side["device"].as_str().unwrap().to_string())
    }

    #[test]
    fn compare_vectors() {
        for case in vectors()["compare"].as_array().unwrap() {
            let (a_time, a_device) = version_of(&case["a"]);
            let (b_time, b_device) = version_of(&case["b"]);
            assert_eq!(
                wins((a_time, &a_device), (b_time, &b_device)),
                case["aWins"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn next_stamp_vectors() {
        for case in vectors()["nextStamp"].as_array().unwrap() {
            let prev = case["prev"].as_u64();
            let now = case["now"].as_u64().unwrap();
            assert_eq!(next_stamp(prev, now), case["stamp"].as_u64().unwrap(), "{}", case["name"]);
        }
    }

    #[test]
    fn apply_remote_vectors() {
        for case in vectors()["applyRemote"].as_array().unwrap() {
            let name = case["name"].to_string();
            let mut ledger = Ledger::new("this-device".into());
            for known in case["versions"].as_array().unwrap() {
                ledger.seed(
                    known["key"].as_str().unwrap(),
                    known["updatedAt"].as_u64().unwrap(),
                    known["device"].as_str().unwrap(),
                    known["deleted"].as_bool().unwrap(),
                );
            }
            let incoming: Vec<Record> = serde_json::from_value(case["incoming"].clone()).unwrap();
            let applied: Vec<String> = ledger.apply_remote(incoming).iter().map(Record::key).collect();
            let expected: Vec<String> =
                case["applied"].as_array().unwrap().iter().map(|key| key.as_str().unwrap().to_string()).collect();
            assert_eq!(applied, expected, "applied: {name}");

            let mut expected_versions: Vec<(String, u64, String, bool)> = case["versionsAfter"]
                .as_array()
                .unwrap()
                .iter()
                .map(|known| {
                    (
                        known["key"].as_str().unwrap().to_string(),
                        known["updatedAt"].as_u64().unwrap(),
                        known["device"].as_str().unwrap().to_string(),
                        known["deleted"].as_bool().unwrap(),
                    )
                })
                .collect();
            expected_versions.sort();
            let actual: Vec<(String, u64, String, bool)> = ledger
                .versions
                .keys()
                .map(|key| {
                    let (updated_at, device, deleted) = ledger.stamp_of(key).unwrap();
                    (key.clone(), updated_at, device, deleted)
                })
                .collect();
            assert_eq!(actual, expected_versions, "versions: {name}");
        }
    }

    fn todo(text: &str) -> Value {
        json!({ "text": text, "createdAt": 1 })
    }

    #[test]
    fn upsert_stamps_and_queues() {
        let mut ledger = Ledger::new("mac".into());
        let record = ledger.local_upsert(TODO, "1", todo("milk"), None, 100).unwrap();
        assert_eq!((record.updated_at, record.device.as_str(), record.deleted), (100, "mac", false));
        assert_eq!(ledger.pending(), 1);
    }

    #[test]
    fn upsert_of_the_same_body_is_not_a_change() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("milk"), None, 100).unwrap();
        assert!(ledger.local_upsert(TODO, "1", todo("milk"), None, 200).is_none());
        // Key order in the body does not matter.
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", json!({ "a": 1, "b": { "c": 2, "d": 3 } }), None, 100).unwrap();
        assert!(ledger.local_upsert(TODO, "1", json!({ "b": { "d": 3, "c": 2 }, "a": 1 }), None, 200).is_none());
    }

    #[test]
    fn an_edit_goes_forward_even_when_the_clock_does_not() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        let record = ledger.local_upsert(TODO, "1", todo("b"), None, 50).unwrap();
        assert_eq!(record.updated_at, 101);
    }

    #[test]
    fn delete_makes_a_tombstone_once() {
        let mut ledger = Ledger::new("mac".into());
        assert!(ledger.local_delete(TODO, "1", 100).is_none(), "never known");
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        let tombstone = ledger.local_delete(TODO, "1", 200).unwrap();
        assert!(tombstone.deleted);
        assert_eq!(tombstone.body, json!({}));
        assert!(ledger.local_delete(TODO, "1", 300).is_none(), "already gone");
    }

    #[test]
    fn a_deleted_record_stays_deleted_when_an_older_version_turns_up() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        ledger.local_delete(TODO, "1", 200).unwrap();
        let late = Record {
            kind: TODO.into(),
            id: "1".into(),
            updated_at: 150,
            device: "phone".into(),
            deleted: false,
            body: todo("a"),
            asset: None,
        };
        assert!(ledger.apply_remote(vec![late]).is_empty());
    }

    #[test]
    fn a_remote_win_clears_what_was_waiting_to_be_sent() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("mine"), None, 100).unwrap();
        let theirs = Record {
            kind: TODO.into(),
            id: "1".into(),
            updated_at: 500,
            device: "phone".into(),
            deleted: false,
            body: todo("theirs"),
            asset: None,
        };
        assert_eq!(ledger.apply_remote(vec![theirs]).len(), 1);
        assert_eq!(ledger.pending(), 0);
    }

    #[test]
    fn the_outbox_is_handed_out_once_and_acknowledged() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        assert_eq!(ledger.take_outbox().len(), 1);
        assert!(ledger.take_outbox().is_empty(), "in flight");
        ledger.acknowledge(&["todo:1".to_string()]);
        assert_eq!(ledger.pending(), 0);
    }

    #[test]
    fn a_restarted_library_gets_everything_again() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        assert_eq!(ledger.take_outbox().len(), 1);
        assert!(ledger.take_outbox().is_empty(), "never sent twice while in flight");
        ledger.release_all();
        assert_eq!(ledger.take_outbox().len(), 1);
    }

    #[test]
    fn a_failed_push_goes_out_again() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        ledger.take_outbox();
        ledger.release(&["todo:1".to_string()]);
        assert_eq!(ledger.take_outbox().len(), 1);
    }

    #[test]
    fn a_change_made_in_flight_survives_the_acknowledgement() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("first"), None, 100).unwrap();
        ledger.take_outbox();
        ledger.local_upsert(TODO, "1", todo("second"), None, 200).unwrap();
        ledger.acknowledge(&["todo:1".to_string()]);
        let next = ledger.take_outbox();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].body["text"], "second");
    }

    #[test]
    fn reconcile_mirrors_only_what_this_device_wrote() {
        let mut ledger = Ledger::new("mac".into());
        let item = |id: &str, text: &str| Desired { id: id.into(), body: json!({ "text": text }), asset: None };
        let first = ledger.reconcile(SESSION, vec![item("a", "1"), item("b", "1")], 100);
        assert_eq!(first.len(), 2);
        // Another device's session shows up through the cloud.
        ledger.apply_remote(vec![Record {
            kind: SESSION.into(),
            id: "other-x".into(),
            updated_at: 90,
            device: "other".into(),
            deleted: false,
            body: json!({}),
            asset: None,
        }]);
        // `a` unchanged, `b` gone, `c` new.
        let second = ledger.reconcile(SESSION, vec![item("a", "1"), item("c", "1")], 200);
        let mut summary: Vec<(String, bool)> = second.iter().map(|record| (record.id.clone(), record.deleted)).collect();
        summary.sort();
        assert_eq!(summary, vec![("b".to_string(), true), ("c".to_string(), false)]);
        assert!(ledger.stamp_of("session:other-x").is_some_and(|(_, _, deleted)| !deleted));
    }

    #[test]
    fn purge_forgets_old_tombstones_only() {
        let mut ledger = Ledger::new("mac".into());
        ledger.seed("todo:old", 100, "x", true);
        ledger.seed("todo:recent", 900, "x", true);
        ledger.seed("todo:live", 100, "x", false);
        ledger.seed("clip:old", 900, "x", true);
        ledger.purge(1_000, |kind| if kind == TODO { 500 } else { 50 });
        assert!(ledger.stamp_of("todo:old").is_none());
        assert!(ledger.stamp_of("todo:recent").is_some());
        assert!(ledger.stamp_of("todo:live").is_some());
        assert!(ledger.stamp_of("clip:old").is_none(), "each kind keeps tombstones for its own time");
    }

    #[test]
    fn live_and_deleted_ids_by_kind() {
        let mut ledger = Ledger::new("mac".into());
        ledger.seed("todo:a", 1, "x", false);
        ledger.seed("todo:b", 1, "x", true);
        ledger.seed("clip:c", 1, "x", false);
        assert_eq!(ledger.live_ids(TODO), vec!["a".to_string()]);
        assert_eq!(ledger.deleted_ids(TODO), vec!["b".to_string()]);
    }

    #[test]
    fn a_tombstone_that_is_still_owed_is_not_purged() {
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        ledger.local_delete(TODO, "1", 200).unwrap();
        ledger.purge(1_000_000, |_| 10);
        assert!(ledger.stamp_of("todo:1").is_some());
    }

    #[test]
    fn the_ledger_survives_a_save_and_a_load() {
        let dir = std::env::temp_dir().join(format!("bangs-ledger-{}", fnv_hex(&format!("{:?}", std::time::SystemTime::now()))));
        let path = dir.join("ledger.json");
        let mut ledger = Ledger::new("mac".into());
        ledger.local_upsert(TODO, "1", todo("a"), None, 100).unwrap();
        ledger.take_outbox();
        ledger.save(&path).unwrap();
        let mut loaded = Ledger::load(&path).unwrap();
        assert_eq!(loaded.device, "mac");
        assert_eq!(loaded.pending(), 1);
        assert_eq!(loaded.take_outbox().len(), 1, "in-flight marks are not saved");
        assert!(Ledger::load(&dir.join("missing.json")).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn records_use_the_wire_format() {
        let record = Record {
            kind: TODO.into(),
            id: "18f2a-0".into(),
            updated_at: 1_760_000_000_000,
            device: "d".into(),
            deleted: false,
            body: json!({ "text": "x" }),
            asset: None,
        };
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["updatedAt"], 1_760_000_000_000u64);
        assert_eq!(wire["asset"], Value::Null);
        assert_eq!(wire["deleted"], false);
        // Extra fields from a newer peer are ignored, missing optional ones default.
        let parsed: Record =
            serde_json::from_str(r#"{"kind":"todo","id":"1","updatedAt":5,"device":"d","extra":1}"#).unwrap();
        assert!(!parsed.deleted && parsed.asset.is_none() && parsed.body.is_null());
    }

    #[test]
    fn ids_are_cloudkit_safe() {
        assert_eq!(clean_id("abc-123_x.y"), "abc-123_x.y");
        assert_eq!(clean_id("会话 1").len(), 16);
        assert_eq!(clean_id(""), fnv_hex(""));
        assert_eq!(clean_id(&"a".repeat(129)).len(), 16);
        assert_eq!(fnv_hex("a"), "af63dc4c8601ec8c");
    }
}
