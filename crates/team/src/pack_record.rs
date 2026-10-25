//! A pack's measured record (#41): success rate and cost per task. The record
//! is local; a shift that holds a key can sign it (`sign_record`) and anyone can
//! check it (`verify_record`). This module does local aggregation (from eval
//! runs or from the shifts' outcomes on the board), the canonical signing
//! payload, signing and verifying, the JSON form and the opt-in gate. It
//! publishes nothing.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::{json, Value};

use crate::board::{Board, Status, Task};
use crate::pack_eval::EvalSummary;

/// One task's outcome, as the measuring shift saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskOutcome {
    /// Hash of the pack the task ran under.
    pub pack_hash: String,
    /// Whether the task succeeded.
    pub succeeded: bool,
    /// Cost of the task, when the shift reported one.
    pub cost: Option<f64>,
    /// Wall-clock seconds the task took.
    pub elapsed_s: f64,
}

/// A pack's measured record. It carries only these fields: no paths, task
/// text, repo names or secrets.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredRecord {
    /// Hash of the measured pack.
    pub pack_hash: String,
    /// Number of tasks measured for this pack.
    pub tasks: u64,
    /// Number of those tasks that succeeded.
    pub successes: u64,
    /// `successes / tasks`, or 0.0 when there are no tasks.
    pub success_rate: f64,
    /// Mean reported cost over the tasks that reported one; None if none did.
    pub cost_per_task: Option<f64>,
    /// Team id of the shift that measured the pack.
    pub measured_by: String,
    /// Signature over [`signing_payload`]; None until the record is signed.
    pub signature: Option<String>,
}

/// One outcome per eval result: `succeeded` is the eval's pass, `cost` the cost
/// the executor reported, `elapsed_s` the eval's time. Pure: publishes nothing.
pub fn outcomes_from_eval(pack_hash: &str, s: &EvalSummary) -> Vec<TaskOutcome> {
    s.per_eval
        .iter()
        .map(|r| TaskOutcome {
            pack_hash: pack_hash.to_string(),
            succeeded: r.passed,
            cost: r.cost,
            elapsed_s: r.elapsed.as_secs_f64(),
        })
        .collect()
}

/// One outcome per task on the board that is Done or Failed and was claimed by
/// one of `workers`. `succeeded` is Done. `cost` is None: an agent's cost on the
/// board is cumulative, not per task. `elapsed_s` is 0.0: the board keeps no
/// task start time. Other statuses and other workers are ignored. Pure.
pub fn outcomes_from_board(pack_hash: &str, workers: &[String], board: &Board) -> Vec<TaskOutcome> {
    board
        .tasks
        .values()
        .filter(|t| matches!(t.status, Status::Done | Status::Failed))
        .filter(|t| t.claimers.iter().any(|c| workers.contains(c)))
        .map(|t| TaskOutcome {
            pack_hash: pack_hash.to_string(),
            succeeded: t.status == Status::Done,
            cost: None,
            elapsed_s: 0.0,
        })
        .collect()
}

/// The tasks of a board `board` verb reply (`{"tasks": {status: [task]}}`) as a
/// `Board` holding just those tasks. Entries that do not parse are skipped.
pub fn board_from_snapshot(snapshot: &Value) -> Board {
    let mut board = Board::default();
    if let Some(groups) = snapshot.get("tasks").and_then(Value::as_object) {
        for list in groups.values().filter_map(Value::as_array) {
            for t in list {
                if let Ok(task) = serde_json::from_value::<Task>(t.clone()) {
                    board.tasks.insert(task.id.clone(), task);
                }
            }
        }
    }
    board
}

/// Aggregate the outcomes that belong to `pack_hash` into a record. Outcomes
/// for other packs are ignored. The record is unsigned.
pub fn aggregate(pack_hash: &str, outcomes: &[TaskOutcome], measured_by: &str) -> MeasuredRecord {
    let mine: Vec<&TaskOutcome> = outcomes
        .iter()
        .filter(|o| o.pack_hash == pack_hash)
        .collect();
    let tasks = mine.len() as u64;
    let successes = mine.iter().filter(|o| o.succeeded).count() as u64;
    let success_rate = if tasks == 0 {
        0.0
    } else {
        successes as f64 / tasks as f64
    };
    let costs: Vec<f64> = mine.iter().filter_map(|o| o.cost).collect();
    let cost_per_task = if costs.is_empty() {
        None
    } else {
        Some(costs.iter().sum::<f64>() / costs.len() as f64)
    };
    MeasuredRecord {
        pack_hash: pack_hash.to_string(),
        tasks,
        successes,
        success_rate,
        cost_per_task,
        measured_by: measured_by.to_string(),
        signature: None,
    }
}

/// The canonical bytes a signature covers: a JSON object with sorted keys and
/// every field except `signature`. The output is deterministic.
pub fn signing_payload(rec: &MeasuredRecord) -> Vec<u8> {
    let mut m: BTreeMap<&str, Value> = BTreeMap::new();
    m.insert("pack_hash", json!(rec.pack_hash));
    m.insert("tasks", json!(rec.tasks));
    m.insert("successes", json!(rec.successes));
    m.insert("success_rate", json!(rec.success_rate));
    m.insert("cost_per_task", json!(rec.cost_per_task));
    m.insert("measured_by", json!(rec.measured_by));
    serde_json::to_vec(&m).unwrap_or_default()
}

/// The record as JSON: exactly the keys of [`signing_payload`] plus `signed`
/// (whether a signature is set). It never carries the signature itself, a path,
/// task text or a secret.
pub fn record_json(rec: &MeasuredRecord) -> Value {
    json!({
        "pack_hash": rec.pack_hash,
        "tasks": rec.tasks,
        "successes": rec.successes,
        "success_rate": rec.success_rate,
        "cost_per_task": rec.cost_per_task,
        "measured_by": rec.measured_by,
        "signed": rec.signature.is_some(),
    })
}

/// A record from its JSON form with the `signature` string added (the shape the
/// index holds). None when a field is missing or has the wrong type.
pub fn record_from_json(v: &Value) -> Option<MeasuredRecord> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(MeasuredRecord {
        pack_hash: text("pack_hash")?,
        tasks: v.get("tasks")?.as_u64()?,
        successes: v.get("successes")?.as_u64()?,
        success_rate: v.get("success_rate")?.as_f64()?,
        cost_per_task: match v.get("cost_per_task") {
            None | Some(Value::Null) => None,
            Some(c) => Some(c.as_f64()?),
        },
        measured_by: text("measured_by")?,
        signature: text("signature"),
    })
}

/// The local record of one eval run (unsigned until [`sign_record`]).
pub fn measured_from_eval(pack_hash: &str, s: &EvalSummary, measured_by: &str) -> MeasuredRecord {
    aggregate(pack_hash, &outcomes_from_eval(pack_hash, s), measured_by)
}

/// Sign a record: `measured_by` becomes the hex public key of `key` (so the
/// shift that measured is the verifying key), then `signature` is the hex
/// ed25519 signature over [`signing_payload`]. Pure.
pub fn sign_record(key: &SigningKey, mut rec: MeasuredRecord) -> MeasuredRecord {
    rec.measured_by = hex::encode(key.verifying_key().to_bytes());
    rec.signature = Some(hex::encode(key.sign(&signing_payload(&rec)).to_bytes()));
    rec
}

/// Check a record's signature against the key in its `measured_by`. Errors are
/// `unsigned`, `bad key` or `bad signature`; they never echo any key material.
pub fn verify_record(rec: &MeasuredRecord) -> Result<(), String> {
    let sig_hex = rec.signature.as_deref().ok_or("unsigned")?;
    let key_bytes: [u8; 32] = hex::decode(&rec.measured_by)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("bad key")?;
    let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| "bad key")?;
    let sig_bytes: [u8; 64] = hex::decode(sig_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("bad signature")?;
    key.verify_strict(&signing_payload(rec), &Signature::from_bytes(&sig_bytes))
        .map_err(|_| "bad signature".to_string())
}

/// The record if it may be published: only when the owner opted in AND the
/// record is signed. Otherwise None.
pub fn publishable(rec: &MeasuredRecord, opt_in: bool) -> Option<MeasuredRecord> {
    if opt_in && rec.signature.is_some() {
        Some(rec.clone())
    } else {
        None
    }
}

/// The record if the owner opted in AND its signature verifies. Otherwise None.
pub fn publishable_verified(rec: &MeasuredRecord, opt_in: bool) -> Option<MeasuredRecord> {
    if opt_in && verify_record(rec).is_ok() {
        Some(rec.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(pack: &str, succeeded: bool, cost: Option<f64>) -> TaskOutcome {
        TaskOutcome {
            pack_hash: pack.to_string(),
            succeeded,
            cost,
            elapsed_s: 1.0,
        }
    }

    #[test]
    fn aggregate_counts_only_this_pack() {
        let outs = vec![
            outcome("p", true, None),
            outcome("q", true, None),
            outcome("q", false, None),
        ];
        let rec = aggregate("p", &outs, "team-a");
        assert_eq!(rec.tasks, 1);
        assert_eq!(rec.successes, 1);
        assert_eq!(rec.pack_hash, "p");
        assert_eq!(rec.measured_by, "team-a");
        assert!(rec.signature.is_none());
    }

    #[test]
    fn rate_and_cost_per_task_are_right() {
        let outs = vec![
            outcome("p", true, Some(1.0)),
            outcome("p", false, Some(3.0)),
            outcome("p", true, None),
        ];
        let rec = aggregate("p", &outs, "team-a");
        assert_eq!(rec.tasks, 3);
        assert_eq!(rec.successes, 2);
        assert!((rec.success_rate - 2.0 / 3.0).abs() < 1e-9);
        // Mean over the two tasks that reported a cost.
        assert_eq!(rec.cost_per_task, Some(2.0));
    }

    #[test]
    fn cost_is_none_when_no_task_reported_cost() {
        let outs = vec![outcome("p", true, None), outcome("p", false, None)];
        let rec = aggregate("p", &outs, "team-a");
        assert_eq!(rec.cost_per_task, None);
    }

    #[test]
    fn an_empty_set_gives_zero_tasks_not_nan() {
        let rec = aggregate("p", &[], "team-a");
        assert_eq!(rec.tasks, 0);
        assert_eq!(rec.successes, 0);
        assert_eq!(rec.success_rate, 0.0);
        assert!(!rec.success_rate.is_nan());
        assert_eq!(rec.cost_per_task, None);
    }

    #[test]
    fn signing_payload_is_deterministic_and_excludes_signature() {
        let outs = vec![outcome("p", true, Some(0.5))];
        let mut rec = aggregate("p", &outs, "team-a");
        let unsigned = signing_payload(&rec);
        assert_eq!(unsigned, signing_payload(&rec));
        rec.signature = Some("sig-one".to_string());
        assert_eq!(unsigned, signing_payload(&rec));
        let text = String::from_utf8(unsigned).unwrap();
        assert!(!text.contains("signature"));
    }

    #[test]
    fn nothing_is_publishable_without_opt_in() {
        let mut rec = aggregate("p", &[outcome("p", true, None)], "team-a");
        rec.signature = Some("sig".to_string());
        assert!(publishable(&rec, false).is_none());
        assert!(publishable(&rec, true).is_some());
    }

    #[test]
    fn an_unsigned_record_is_not_publishable() {
        let rec = aggregate("p", &[outcome("p", true, None)], "team-a");
        assert!(publishable(&rec, true).is_none());
    }

    #[test]
    fn outcomes_from_eval_matches_summary() {
        use crate::pack_eval::{EvalResult, EvalSummary};
        use std::time::Duration;
        let r = |id: &str, passed: bool, cost: Option<f64>| EvalResult {
            id: id.to_string(),
            passed,
            reason: None,
            elapsed: Duration::from_millis(1500),
            cost,
        };
        let s = EvalSummary {
            passed: 2,
            total: 3,
            pass_rate: 2.0 / 3.0,
            total_time: Duration::from_millis(4500),
            total_cost: Some(0.5),
            per_eval: vec![
                r("a", true, Some(0.5)),
                r("b", false, None),
                r("c", true, None),
            ],
        };
        let outs = outcomes_from_eval("p", &s);
        assert_eq!(outs.len(), 3);
        assert!(!outs[1].succeeded, "a failed eval is a failed outcome");
        assert_eq!(outs[0].cost, Some(0.5));
        assert_eq!(outs[0].elapsed_s, 1.5);
        let rec = aggregate("p", &outs, "team-a");
        assert_eq!(rec.tasks as usize, s.total);
        assert_eq!(rec.successes as usize, s.passed);
        assert!((rec.success_rate - s.pass_rate).abs() < 1e-9);
    }

    #[test]
    fn a_record_carries_no_task_text_or_paths() {
        let rec = aggregate("p", &[outcome("p", true, Some(1.0))], "team-a");
        let v: Value = serde_json::from_slice(&signing_payload(&rec)).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "cost_per_task",
                "measured_by",
                "pack_hash",
                "success_rate",
                "successes",
                "tasks",
            ]
        );
    }

    fn summary3() -> EvalSummary {
        use crate::pack_eval::EvalResult;
        use std::time::Duration;
        let r = |id: &str, passed: bool, cost: Option<f64>| EvalResult {
            id: id.to_string(),
            passed,
            reason: None,
            elapsed: Duration::from_millis(10),
            cost,
        };
        EvalSummary {
            passed: 2,
            total: 3,
            pass_rate: 2.0 / 3.0,
            total_time: Duration::from_millis(30),
            total_cost: Some(4.0),
            per_eval: vec![
                r("a", true, Some(1.0)),
                r("b", false, None),
                r("c", true, Some(3.0)),
            ],
        }
    }

    #[test]
    fn record_json_has_exactly_the_signed_fields_plus_signed_flag() {
        let mut rec = aggregate("p", &[outcome("p", true, Some(0.5))], "team-a");
        let v = record_json(&rec);
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "cost_per_task",
                "measured_by",
                "pack_hash",
                "signed",
                "success_rate",
                "successes",
                "tasks",
            ]
        );
        assert_eq!(v["signed"], false);
        assert_eq!(v["pack_hash"], "p");
        assert_eq!(v["tasks"], 1);
        assert_eq!(v["successes"], 1);
        assert_eq!(v["success_rate"], 1.0);
        assert_eq!(v["cost_per_task"], 0.5);
        assert_eq!(v["measured_by"], "team-a");
        rec.signature = Some("sig".to_string());
        let v = record_json(&rec);
        assert_eq!(v["signed"], true);
        assert!(!v.to_string().contains("sig\""), "{v}");
    }

    #[test]
    fn measured_from_eval_matches_aggregate_of_outcomes() {
        let s = summary3();
        let rec = measured_from_eval("p", &s, "team-a");
        assert_eq!(rec.tasks, 3);
        assert_eq!(rec.successes, 2);
        assert!((rec.success_rate - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(rec.cost_per_task, Some(2.0));
        assert!(rec.signature.is_none());
        assert_eq!(rec, aggregate("p", &outcomes_from_eval("p", &s), "team-a"));
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn signed() -> MeasuredRecord {
        let outs = vec![outcome("p", true, Some(1.0)), outcome("p", false, None)];
        sign_record(&key(1), aggregate("p", &outs, ""))
    }

    #[test]
    fn signed_record_verifies() {
        let rec = signed();
        assert!(rec.signature.is_some());
        assert_eq!(
            rec.measured_by,
            hex::encode(key(1).verifying_key().to_bytes())
        );
        assert_eq!(verify_record(&rec), Ok(()));
    }

    #[test]
    fn a_changed_field_fails_verification() {
        let ok = signed();
        let mut cases: Vec<MeasuredRecord> = Vec::new();
        let mut r = ok.clone();
        r.tasks += 1;
        cases.push(r);
        let mut r = ok.clone();
        r.successes += 1;
        cases.push(r);
        let mut r = ok.clone();
        r.success_rate = 1.0;
        cases.push(r);
        let mut r = ok.clone();
        r.cost_per_task = Some(9.0);
        cases.push(r);
        let mut r = ok.clone();
        r.pack_hash = "q".into();
        cases.push(r);
        for r in cases {
            assert_eq!(verify_record(&r), Err("bad signature".to_string()), "{r:?}");
        }
    }

    #[test]
    fn a_different_signer_key_fails() {
        let mut rec = signed();
        rec.measured_by = hex::encode(key(2).verifying_key().to_bytes());
        assert!(verify_record(&rec).is_err());
        rec.measured_by = "nothex".into();
        assert_eq!(verify_record(&rec), Err("bad key".to_string()));
    }

    #[test]
    fn an_unsigned_record_does_not_verify() {
        let rec = aggregate("p", &[outcome("p", true, None)], "x");
        assert_eq!(verify_record(&rec), Err("unsigned".to_string()));
    }

    fn board_with(tasks: &[(&str, Status, &[&str])]) -> Board {
        let mut b = Board::default();
        for (id, status, claimers) in tasks {
            b.tasks.insert(
                id.to_string(),
                Task {
                    id: id.to_string(),
                    status: *status,
                    claimers: claimers.iter().map(|c| c.to_string()).collect(),
                    ..Task::default()
                },
            );
        }
        b
    }

    fn shift_board() -> Board {
        board_with(&[
            ("o/r#1", Status::Done, &["w1"]),
            ("o/r#2", Status::Failed, &["w1", "other"]),
            ("o/r#3", Status::Pending, &["w1"]),
            ("o/r#4", Status::InWork, &["w1"]),
            ("o/r#5", Status::Done, &["foreign"]),
        ])
    }

    #[test]
    fn outcomes_from_board_counts_done_and_failed_for_the_workers() {
        let outs = outcomes_from_board("p", &["w1".to_string()], &shift_board());
        assert_eq!(outs.len(), 2);
        assert_eq!(outs.iter().filter(|o| o.succeeded).count(), 1);
        assert!(outs.iter().all(|o| o.cost.is_none() && o.elapsed_s == 0.0));
        assert!(outs.iter().all(|o| o.pack_hash == "p"));
    }

    #[test]
    fn aggregate_over_shift_outcomes_gives_rate() {
        let outs = outcomes_from_board("p", &["w1".to_string()], &shift_board());
        let rec = aggregate("p", &outs, "");
        assert_eq!((rec.tasks, rec.successes), (2, 1));
        assert_eq!(rec.success_rate, 0.5);
    }

    #[test]
    fn board_from_snapshot_reads_tasks_of_every_status() {
        let b = shift_board();
        let back = board_from_snapshot(&b.snapshot());
        assert_eq!(back.tasks, b.tasks);
    }

    #[test]
    fn record_from_json_round_trips_a_signed_record() {
        let rec = signed();
        let mut v = record_json(&rec);
        v["signature"] = json!(rec.signature.clone().unwrap());
        let back = record_from_json(&v).unwrap();
        assert_eq!(back, rec);
        assert_eq!(verify_record(&back), Ok(()));
        assert!(record_from_json(&json!({"tasks": 1})).is_none());
    }

    #[test]
    fn publishable_verified_requires_opt_in_and_a_valid_signature() {
        let rec = signed();
        assert!(publishable_verified(&rec, false).is_none());
        let mut unsigned = rec.clone();
        unsigned.signature = None;
        assert!(publishable_verified(&unsigned, true).is_none());
        let mut tampered = rec.clone();
        tampered.tasks = 99;
        assert!(publishable_verified(&tampered, true).is_none());
        let mut fake = rec.clone();
        fake.signature = Some("sig".into());
        assert!(
            publishable_verified(&fake, true).is_none(),
            "publishable() would accept"
        );
        assert_eq!(publishable_verified(&rec, true), Some(rec));
    }
}
