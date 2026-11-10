//! Cadencer feed: the pure data side of the dispatcher's cadencer. It writes the
//! load line of a digest and meters what the digests cost per hour.
//! Nothing here reads a clock, sends a message or touches the store.

use serde_json::{json, Value};

/// Seconds the cost meter reports over.
const WINDOW_S: u64 = 3600;

/// One plain line of the board's load counts, for a digest. Numbers only: no
/// mode, no bar, no "x / N". `None` until the board has given task counts.
pub fn load_line(load: &Value) -> Option<String> {
    let tasks = load.get("tasks")?;
    let workers = &load["workers"];
    let n = |v: &Value, k: &str| v[k].as_u64().unwrap_or(0);
    Some(format!(
        "load: busy {} idle {} stalled {} · held {} in_review {} done {} blocked {} queued {} · done/h {}",
        n(workers, "busy"),
        n(workers, "idle"),
        n(workers, "stalled"),
        n(tasks, "held"),
        n(tasks, "in_review"),
        n(tasks, "done"),
        n(tasks, "blocked"),
        n(tasks, "queued"),
        n(load, "done_last_hour"),
    ))
}

/// Digest bytes over the last hour. Times are passed in.
#[derive(Debug, Clone, Default)]
pub struct CostMeter {
    digests: Vec<(u64, u64)>,
}

fn in_window(at: u64, now_s: u64) -> bool {
    now_s.saturating_sub(at) < WINDOW_S
}

fn window_sum(entries: &[(u64, u64)], now_s: u64) -> u64 {
    entries
        .iter()
        .filter(|(at, _)| in_window(*at, now_s))
        .map(|(_, v)| v)
        .sum()
}

impl CostMeter {
    /// An empty meter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one digest of `bytes` sent at `now_s`.
    pub fn record_digest(&mut self, now_s: u64, bytes: u64) {
        self.digests.push((now_s, bytes));
        self.digests.retain(|(at, _)| in_window(*at, now_s));
    }

    /// The per-hour figure as of `now_s`.
    pub fn status(&self, now_s: u64) -> Value {
        json!({
            "digest_bytes_per_hour": window_sum(&self.digests, now_s),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_per_hour_is_reported_in_status() {
        let mut m = CostMeter::new();
        m.record_digest(1000, 100);
        m.record_digest(1000, 50);
        assert_eq!(m.status(1000), json!({"digest_bytes_per_hour": 150}));
        assert_eq!(m.status(1000 + 3601), json!({"digest_bytes_per_hour": 0}));
    }
}
