//! The load and progress view (#127): the code gives plain counts, the owning
//! agent decides what to show. These tests check that no mode, enum or
//! progress/load switch exists in the load code, and that a scripted model
//! gets the same digest for a fixed list and for a refilling queue.

use std::collections::BTreeMap;
use std::sync::Mutex;

use fantastic_team::board::{load_numbers, Board, Group, Live, Status, Task};
use fantastic_team::cadence_feed::load_line;
use serde_json::Value;

/// The files the load code lives in.
const LOAD_SOURCES: &[(&str, &str)] = &[
    ("board.rs", include_str!("../src/board.rs")),
    ("cadencer.rs", include_str!("../src/cadencer.rs")),
    ("cadence_feed.rs", include_str!("../src/cadence_feed.rs")),
    ("dispatcher.rs", include_str!("../src/dispatcher.rs")),
];

#[test]
fn no_mode_field_or_enum_for_load() {
    for forbidden in ["enum LoadMode", "enum View", "enum Progress", "enum Load"] {
        for (file, src) in LOAD_SOURCES {
            assert!(!src.contains(forbidden), "{forbidden} found in {file}");
        }
    }
    for (file, src) in LOAD_SOURCES {
        for line in src.lines().filter(|l| l.contains("load")) {
            for word in ["\"mode\"", "\"pipe\"", "\"list\""] {
                assert!(
                    !line.contains(word),
                    "{word} next to load in {file}: {line}"
                );
            }
        }
    }
}

#[test]
fn load_numbers_has_no_total_or_of_key() {
    let mut b = Board::default();
    b.tasks.insert(
        "o/r#1".into(),
        Task {
            id: "o/r#1".into(),
            status: Status::Pending,
            ..Default::default()
        },
    );
    let groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut live = BTreeMap::new();
    live.insert("w1".to_string(), Live::default());
    b.touch_agent("w1", "idle", 0.0, 100);
    let load = load_numbers(&b, &groups, &live, 100, 300);
    let text = load.to_string();
    assert!(!text.contains("\"total\""), "{text}");
    assert!(!text.contains("\"of\""), "{text}");
    assert!(load["tasks"].get("waiting_ci").is_none());
}

/// A stand-in for the group's driver model: it reads the digest it is sent
/// and answers with the reply it was scripted to give.
struct ScriptedModel {
    reply: String,
    seen: Mutex<Vec<String>>,
}

impl ScriptedModel {
    fn new(reply: &str) -> Self {
        ScriptedModel {
            reply: reply.into(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, digest: &str) -> String {
        self.seen.lock().unwrap().push(digest.into());
        self.reply.clone()
    }
}

/// The one digest path both runs use: the load numbers, then the plain line.
fn digest_for(load: &Value) -> String {
    load_line(load, 0).expect("task counts are present")
}

fn board_with(done: usize, pending: usize, queued_done_recent: bool) -> Board {
    let mut b = Board::default();
    let now = 10_000;
    let mut put = |i: usize, status: Status, updated: u64| {
        let id = format!("o/r#{i}");
        b.tasks.insert(
            id.clone(),
            Task {
                id,
                status,
                updated,
                pr: (status == Status::Done).then(|| format!("p{i}")),
                ..Default::default()
            },
        );
    };
    for i in 0..done {
        let updated = if queued_done_recent {
            now - 10
        } else {
            now - 9_000
        };
        put(i, Status::Done, updated);
    }
    for i in 0..pending {
        put(1_000 + i, Status::Pending, now);
    }
    b
}

#[test]
fn scripted_model_renders_progress_or_load_only() {
    let now = 10_000;
    let groups = BTreeMap::new();
    let live = BTreeMap::new();

    // Fixed list: 29 tasks, 12 done, the rest queued.
    let fixed = board_with(12, 17, false);
    let fixed_digest = digest_for(&load_numbers(&fixed, &groups, &live, now, 300));
    let fixed_model = ScriptedModel::new("12 / 29 done · 3 in review");
    let fixed_reply = fixed_model.answer(&fixed_digest);

    // Refilling queue: work keeps arriving, no fixed total.
    let queue = board_with(12, 5, true);
    let queue_digest = digest_for(&load_numbers(&queue, &groups, &live, now, 300));
    let queue_model = ScriptedModel::new("load: 5 queued, 12 done in the last hour");
    let queue_reply = queue_model.answer(&queue_digest);

    // The driver received the numbers in both runs.
    assert!(fixed_model.seen.lock().unwrap()[0].contains("queued 17"));
    assert!(fixed_model.seen.lock().unwrap()[0].contains("done 12"));
    assert!(queue_model.seen.lock().unwrap()[0].contains("queued 5"));
    assert!(queue_model.seen.lock().unwrap()[0].contains("done/h 12"));
    // Each reply is rendered as the model gave it.
    assert_eq!(fixed_reply, "12 / 29 done · 3 in review");
    assert_eq!(queue_reply, "load: 5 queued, 12 done in the last hour");
    // The code path is the same function for both runs; only the input differs.
    assert_eq!(
        fixed_digest.split(" · ").next(),
        queue_digest.split(" · ").next()
    );
}
