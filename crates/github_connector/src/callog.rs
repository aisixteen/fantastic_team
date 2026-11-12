//! A short in-memory log of the verbs this connector served, and the last rate
//! limit GitHub reported. Both feed the `reflect` reply so a page can show them.
//! An entry holds only the verb, the target the caller named (`owner/name` or
//! `owner/name#N`) and a coarse result. It never holds a reply, a header value
//! other than the three rate numbers, or a token.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// How many calls are kept per agent.
pub(crate) const CAP: usize = 50;

#[derive(Clone)]
struct Entry {
    verb: String,
    target: String,
    result: String,
}

#[derive(Default)]
struct State {
    calls: HashMap<String, VecDeque<Entry>>,
    rates: HashMap<String, (u64, u64, u64)>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}

fn key(id: &str, api_url: &str) -> String {
    format!("{id}@{api_url}")
}

/// The target a caller named, read from the request and never from the reply.
pub(crate) fn target_of(call: &crate::verbs::Call) -> String {
    use crate::verbs::Call;
    // `staleness` names its repos inside `versions` or `issue`: log the first
    let (repo, number): (&str, Option<u64>) = match call {
        Call::PrCreate(a) => (&a.repo, None),
        Call::Issues(a) => (&a.repo, None),
        Call::IssueCreate(a) => (&a.repo, None),
        Call::ProjectFieldSet(a) => (&a.repo, None),
        Call::ReadFile(a) => (&a.repo, None),
        Call::IssueView(a) => (&a.repo, Some(a.number)),
        Call::IssueEdit(a) => (&a.repo, Some(a.number)),
        Call::IssueState(a) => (&a.repo, Some(a.number)),
        Call::LabelEvents(a) => (&a.repo, Some(a.number)),
        Call::Comment(a) => (&a.repo, Some(a.number)),
        Call::Label(a) => (&a.repo, Some(a.number)),
        Call::PrStatus(a) => (&a.repo, Some(a.number)),
        Call::Merge(a) => (&a.repo, Some(a.number)),
        Call::Staleness(a) => match (a.versions.as_ref().and_then(|v| v.first()), &a.issue) {
            (Some(v), _) => (&v.repo, None),
            (None, Some(i)) => (&i.repo, None),
            (None, None) => ("", None),
        },
        _ => ("", None),
    };
    let repo: String = repo.chars().take(100).collect();
    match number {
        Some(n) if !repo.is_empty() => format!("{repo}#{n}"),
        _ => repo,
    }
}

/// The coarse result of a reply: ok, refused, unreachable or `error <status>`.
pub(crate) fn result_of(reply: &Value) -> String {
    if reply.get("transport").is_some() {
        return "unreachable".to_string();
    }
    if let Some(s) = reply.get("status").and_then(|s| s.as_u64()) {
        if (100..1000).contains(&s) && reply.get("error").is_some() {
            return format!("error {s}");
        }
    }
    if reply.get("error").is_some() {
        return "refused".to_string();
    }
    "ok".to_string()
}

/// Record one served call.
pub(crate) fn push(id: &str, api_url: &str, verb: &str, target: String, result: String) {
    let mut s = state().lock().unwrap();
    let q = s.calls.entry(key(id, api_url)).or_default();
    q.push_back(Entry {
        verb: verb.to_string(),
        target,
        result,
    });
    while q.len() > CAP {
        q.pop_front();
    }
}

/// The kept calls, oldest first, as `[{verb, target, result}]`.
pub(crate) fn recent(id: &str, api_url: &str) -> Value {
    let s = state().lock().unwrap();
    let v: Vec<Value> = s
        .calls
        .get(&key(id, api_url))
        .map(|q| {
            q.iter()
                .map(|e| json!({"verb": e.verb, "target": e.target, "result": e.result}))
                .collect()
        })
        .unwrap_or_default();
    Value::Array(v)
}

/// Store the rate triple GitHub sent for this API root.
pub(crate) fn set_rate(api_url: &str, limit: u64, remaining: u64, reset: u64) {
    state()
        .lock()
        .unwrap()
        .rates
        .insert(api_url.to_string(), (limit, remaining, reset));
}

/// The last rate triple as `{limit, remaining, reset}`, or null if none was seen.
pub(crate) fn rate(api_url: &str) -> Value {
    match state().lock().unwrap().rates.get(api_url) {
        Some((l, r, t)) => json!({"limit": l, "remaining": r, "reset": t}),
        None => Value::Null,
    }
}

/// Forget the calls and the rate of one agent: it has just booted.
pub(crate) fn reset(id: &str, api_url: &str) {
    let mut s = state().lock().unwrap();
    s.calls.remove(&key(id, api_url));
    s.rates.remove(api_url);
}
