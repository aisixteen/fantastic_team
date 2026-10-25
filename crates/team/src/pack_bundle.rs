//! Worker packs: `pack` a worker into a directory, `install` it as a template.
//!
//! A pack is `pack.yaml` (the manifest: name, version, author, license, price,
//! the content hash of the other files, ports and the file list, see
//! `pack_manifest`), `manifest.yaml` (name and version), `worker.json` (the
//! worker record with bindings stripped), and `directives/`, `templates/`,
//! `evals/`. `pack` runs the secrets scan (`pack_scan`) before it writes.
//! `install` checks the manifest, the content hash and the scan before it
//! copies the pack under the canvas templates dir as a TEMPLATE. It never
//! creates an agent: every effect goes through the `files` agent.
//!
//! `install {pack, into, bindings?}` also takes the buyer's bindings: a map from
//! each port key (`model`, `connector:<name>`, `exec:sandboxed`) to the id of
//! the agent that fills it. They are checked before the first write and kept in
//! `<dest>/bindings.json`. The reply says which ports are `bound` and which are
//! `unbound`, so the caller can ask the buyer and re-run. Install never gates on
//! unbound ports; enforcing bindings at enable time is not built.
//!
//! `eval` runs a pack's evals on demand against a worker and reports the pass
//! rate, time and cost. `install` runs them too, but only when it is given a
//! worker (`install {pack, into, worker}`): after the copy it reports the same
//! report under `evals`, and an advisory `enable {allowed, min_pass_rate,
//! reason}` verdict (`min_pass_rate` defaults to 1.0). Install never fails or
//! rolls back on the verdict, and creates no agent, so nothing enforces it yet:
//! enforcing the verdict where a worker is created from a template is not built.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::pack_eval::{load_evals, run_evals, summary_text, EvalSummary, PeersExecutor};
use crate::pack_manifest;
use crate::pack_record::{measured_from_eval, record_json};
use crate::pack_scan;
use crate::puller::{KernelPeers, Peers};

/// The per-eval timeout when `eval` is not given `timeout_s`.
pub const DEFAULT_EVAL_TIMEOUT_S: f64 = 60.0;

/// Handler module name of the pack bundle.
pub const HANDLER_MODULE: &str = "team_pack.tools";

/// Keys that bind a worker to a backend, model or credential. Never packed.
/// Matched by exact key name, case-insensitively: a secret kept under any
/// other key name is NOT stripped, so a worker record must not hold one.
const BINDING_KEYS: &[&str] = &[
    "backend",
    "backend_id",
    "model",
    "model_id",
    "credentials",
    "credential",
    "api_key",
    "token",
    "access_token",
    "secret",
    "secrets",
    "client_secret",
    "auth",
    "authorization",
    "password",
    "passphrase",
    "private_key",
];

fn is_binding_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    BINDING_KEYS.contains(&k.as_str())
}

/// The directories a pack carries, besides the manifest and the record.
const PACK_DIRS: &[&str] = &["directives", "templates", "evals"];

/// Remove binding and credential keys from a worker record, recursively.
pub fn strip_bindings(record: &Value) -> Value {
    match record {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !is_binding_key(k))
                .map(|(k, v)| (k.clone(), strip_bindings(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_bindings).collect()),
        other => other.clone(),
    }
}

/// The fixed files of a pack, relative to the pack directory.
pub fn layout_paths() -> Vec<String> {
    let mut out = vec!["manifest.yaml".to_string(), "worker.json".to_string()];
    for d in PACK_DIRS {
        out.push(format!("{d}/.keep"));
    }
    out
}

/// Where an installed pack lands: `<into>/<name>-<version>/`.
pub fn install_dir(into: &Path, name: &str, version: &str) -> PathBuf {
    into.join(format!("{name}-{version}"))
}

/// The key of an installed pack's directory, as a string: `<into>/<name>-<version>`.
/// Built with `/` by hand, not from a `PathBuf`: the key is sent to the peer
/// file verbs and echoed back as `path`, so it is a portable `/` path like every
/// other path in this module, never an OS-rendered one (`\\` on Windows).
fn install_dest(into: &str, name: &str, version: &str) -> String {
    let dir = format!("{name}-{version}");
    if into.is_empty() {
        dir
    } else if into.ends_with('/') {
        format!("{into}{dir}")
    } else {
        format!("{into}/{dir}")
    }
}

/// A name or version that is safe as one path segment and one manifest line:
/// letters, digits, `.`, `_`, `-`, `+`; never empty, never `..`.
pub fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

/// The manifest text: minimal, name and version only.
pub fn manifest_yaml(name: &str, version: &str) -> String {
    format!("name: {name}\nversion: {version}\n")
}

/// Read `name` and `version` back out of a manifest written by `manifest_yaml`.
pub fn parse_manifest(text: &str) -> Option<(String, String)> {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
            .map(|v| v.trim().to_string())
    };
    Some((field("name")?, field("version")?))
}

fn err(msg: &str) -> Value {
    json!({"error": msg})
}

/// Text of a `files` read reply: `content`, or the joined `lines` of a listing.
fn text_of(reply: &Value) -> Option<String> {
    if reply.get("error").is_some() {
        return None;
    }
    if let Some(c) = reply.get("content").and_then(Value::as_str) {
        return Some(c.to_string());
    }
    let lines = reply.get("lines")?.as_array()?;
    Some(
        lines
            .iter()
            .filter_map(|l| l.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Read a whole text file through the file bridge. The bridge answers with
/// pages of `lines` (200 a page, `eof` on the last), so the pages are read in
/// turn and joined with newlines. The file's last newline is not kept by the
/// bridge, which is why `pack` writes its files without one.
pub(crate) async fn read<P: Peers>(p: &P, path: &str) -> Option<String> {
    let mut text = String::new();
    let mut offset = 0u64;
    loop {
        let r = p
            .ask(
                "files",
                json!({"type": "read", "path": path, "offset": offset}),
            )
            .await;
        let page = text_of(&r)?;
        let Some(lines) = r.get("lines").and_then(Value::as_array) else {
            return Some(page);
        };
        if offset > 0 {
            text.push('\n');
        }
        text.push_str(&page);
        offset += lines.len() as u64;
        if r.get("eof").and_then(Value::as_bool).unwrap_or(true) || lines.is_empty() {
            return Some(text);
        }
    }
}

async fn write<P: Peers>(p: &P, path: &str, bytes: &str) -> bool {
    let r = p
        .ask(
            "files",
            json!({"type": "write", "path": path, "bytes": bytes}),
        )
        .await;
    r.get("error").is_none()
}

/// Entry names of a directory, from a `files` list reply.
pub(crate) async fn list<P: Peers>(p: &P, path: &str) -> Vec<String> {
    let r = p.ask("files", json!({"type": "list", "path": path})).await;
    if let Some(a) = r.get("files").and_then(Value::as_array) {
        return a
            .iter()
            .filter_map(|f| f.get("name").and_then(Value::as_str))
            .map(String::from)
            .collect();
    }
    r.get("entries")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Does a directory exist? A `files` list of it answers without an error.
async fn exists<P: Peers>(p: &P, path: &str) -> bool {
    p.ask("files", json!({"type": "list", "path": path}))
        .await
        .get("error")
        .is_none()
}

/// Write `<root>/<rel>`; record it under `ok`, or under `failed` when the write fails.
async fn put<P: Peers>(
    p: &P,
    root: &str,
    rel: &str,
    text: &str,
    ok: &mut Vec<String>,
    failed: &mut Vec<String>,
) {
    if write(p, &format!("{root}/{rel}"), text).await {
        ok.push(rel.to_string());
    } else {
        failed.push(rel.to_string());
    }
}

/// A parsed port as the install reply shows it: `{kind, ...}`.
fn port_json(port: &pack_manifest::Port) -> Value {
    use pack_manifest::{Port, Requirement};
    match port {
        Port::Model(reqs) => {
            let window = reqs.iter().find_map(|r| match r {
                Requirement::Window(n) => Some(*n),
                Requirement::Tools => None,
            });
            let tools = reqs.iter().any(|r| matches!(r, Requirement::Tools));
            json!({"kind": "model", "window": window, "tools": tools})
        }
        Port::Connector(name) => json!({"kind": "connector", "name": name}),
        Port::ExecSandboxed => json!({"kind": "exec:sandboxed"}),
    }
}

/// The stable key of a port: `model`, `connector:<name>` or `exec:sandboxed`.
/// Two `@model` ports share the key `model`, so a binding for it fills both.
pub fn port_key(port: &pack_manifest::Port) -> String {
    use pack_manifest::Port;
    match port {
        Port::Model(_) => "model".to_string(),
        Port::Connector(name) => format!("connector:{name}"),
        Port::ExecSandboxed => "exec:sandboxed".to_string(),
    }
}

/// The buyer's bindings: a map from port key to the id of the agent that fills it.
/// `Null` (no bindings given) is the empty map. Refuses a non-object, a key that
/// is not a port key of the pack (the error names it), and a value that is not a
/// safe agent id: a non-empty string, `valid_part`, and not key-shaped. The value
/// is never echoed in the error, so a mistyped secret is not repeated back.
pub fn check_bindings(
    bindings: &Value,
    ports: &[pack_manifest::Port],
) -> Result<BTreeMap<String, String>, String> {
    let obj = match bindings {
        Value::Null => return Ok(BTreeMap::new()),
        Value::Object(obj) => obj,
        _ => return Err("bindings must be an object of {port key: agent id}".to_string()),
    };
    let keys: BTreeSet<String> = ports.iter().map(port_key).collect();
    let mut out = BTreeMap::new();
    for (key, value) in obj {
        if !keys.contains(key) {
            return Err(format!("binding key `{key}` is not a port of this pack"));
        }
        let id = value
            .as_str()
            .ok_or_else(|| format!("binding `{key}` must be an agent id string"))?;
        if !valid_part(id) || pack_manifest::looks_like_key(id) {
            return Err(format!("binding `{key}` is not a safe agent id"));
        }
        out.insert(key.clone(), id.to_string());
    }
    Ok(out)
}

/// The port keys of a pack in order, each once, that no binding fills.
fn unbound_keys(ports: &[pack_manifest::Port], bound: &BTreeMap<String, String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ports
        .iter()
        .map(port_key)
        .filter(|k| seen.insert(k.clone()) && !bound.contains_key(k))
        .collect()
}

/// The scan refusal reply: the findings with path, rule and line only.
pub(crate) fn refused_by_scan(report: &pack_scan::ScanReport) -> Value {
    let findings: Vec<Value> = report
        .findings
        .iter()
        .map(|f| json!({"path": f.path, "rule": f.rule, "line": f.line}))
        .collect();
    json!({"error": "pack refused by scan", "findings": findings})
}

/// Pack the worker whose record is `<worker>/worker.json` into the directory `out`.
/// Refuses any existing `out` (a directory with anything in it or empty), a worker
/// with no name, a name or version that is not a safe path segment, and a pack the
/// secrets scan refuses. Writes `pack.yaml` (the manifest, with the content hash
/// of every other packed file) and `manifest.yaml`. Reports every failed write.
pub async fn pack<P: Peers>(p: &P, worker: &str, out: &str) -> Value {
    if exists(p, out).await {
        return err("out already exists");
    }
    let Some(raw) = read(p, &format!("{worker}/worker.json")).await else {
        return err("worker record not found");
    };
    let Ok(record) = serde_json::from_str::<Value>(&raw) else {
        return err("worker record is not json");
    };
    let name = match record.get("name").and_then(Value::as_str) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return err("worker has no name"),
    };
    let version = record
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("0.1.0")
        .to_string();
    if !valid_part(&name) || !valid_part(&version) {
        return err("worker name or version is not a safe path segment");
    }
    let Some(port_strings) = record.get("ports").map_or(Some(vec![]), |v| {
        v.as_array()?
            .iter()
            .map(|x| x.as_str().map(String::from))
            .collect::<Option<Vec<String>>>()
    }) else {
        return err("worker ports must be strings");
    };
    let text_field = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string()
    };
    let stripped = strip_bindings(&record);

    // Gather every packed file first. Nothing is written until all checks pass.
    let mut gathered: Vec<(String, String)> = vec![
        ("manifest.yaml".to_string(), manifest_yaml(&name, &version)),
        (
            "worker.json".to_string(),
            serde_json::to_string_pretty(&stripped).unwrap_or_default(),
        ),
    ];
    for d in PACK_DIRS {
        let keep = format!("{d}/.keep");
        if !gathered.iter().any(|(p, _)| *p == keep) {
            gathered.push((keep, String::new()));
        }
        for entry in list(p, &format!("{worker}/{d}")).await {
            let rel = format!("{d}/{entry}");
            if gathered.iter().any(|(p, _)| *p == rel) {
                continue;
            }
            if let Some(text) = read(p, &format!("{worker}/{rel}")).await {
                gathered.push((rel, text));
            }
        }
    }
    // The bridge does not return a file's last newline, so none is packed: what
    // `install` reads back is then exactly what was hashed here.
    for (_, t) in gathered.iter_mut() {
        *t = t.trim_end_matches('\n').to_string();
    }
    let bytes: Vec<(String, Vec<u8>)> = gathered
        .iter()
        .map(|(p, t)| (p.clone(), t.clone().into_bytes()))
        .collect();
    let report = pack_scan::scan(&bytes);
    if report.refused {
        return refused_by_scan(&report);
    }
    let manifest = pack_manifest::Manifest {
        name: name.clone(),
        version: version.clone(),
        author: text_field("author"),
        license: text_field("license"),
        price: pack_manifest::FREE.to_string(),
        content_hash: pack_manifest::content_hash(&bytes),
        signature: String::new(),
        ports: port_strings,
        files: gathered.iter().map(|(p, _)| p.clone()).collect(),
    };
    if let Err(e) = manifest.validate() {
        return err(&e);
    }
    let Ok(manifest_text) = serde_yaml::to_string(&manifest) else {
        return err("pack manifest could not be written");
    };
    gathered.push((pack_manifest::MANIFEST_FILE.to_string(), manifest_text));

    let mut written = Vec::new();
    let mut failed = Vec::new();
    for (rel, text) in &gathered {
        put(p, out, rel, text, &mut written, &mut failed).await;
    }
    if !failed.is_empty() {
        return json!({"error": "write failed", "failed": failed, "files": written});
    }
    json!({"packed": out, "name": name, "version": version, "files": written})
}

/// Install the pack at `pack` as a template under `into`, as `<name>-<version>/`.
/// Refuses a pack with no `pack.yaml`, an invalid manifest, an existing version,
/// a name or version that is not a safe path segment (no `/`, `\`, `..`), a file
/// set whose content hash differs from the manifest, and a pack the secrets scan
/// refuses. Every check runs before the first write. Creates no agent.
///
/// With a non-empty `worker`, the copied pack's evals run on it after the copy
/// and the report goes under `evals` (or `skipped` when the pack has none, or
/// `error` when they cannot be loaded). `enable_verdict` of that report, against
/// `min_pass_rate`, goes under `enable`. Neither the report nor the verdict
/// fails the install or rolls it back; the verdict is advice for the caller.
///
/// `bindings` (`{port key: agent id}`, or `Null`) fills the pack's ports. It is
/// checked by `check_bindings` before the first write; the bound map is written
/// to `<dest>/bindings.json` (the installer's data, not hashed) and the reply
/// carries `bindings: {bound, unbound}`. Unbound ports never fail the install:
/// the caller asks the user and re-runs with the map.
pub async fn install<P: Peers>(
    p: &P,
    pack: &str,
    into: &str,
    worker: &str,
    timeout: Duration,
    bindings: &Value,
    min_pass_rate: f64,
) -> Value {
    let Some(text) = read(p, &format!("{pack}/{}", pack_manifest::MANIFEST_FILE)).await else {
        return err("pack has no pack.yaml");
    };
    let manifest = match pack_manifest::Manifest::parse(&text) {
        Ok(m) => m,
        Err(e) => return err(&format!("pack.yaml: {e}")),
    };
    if let Err(e) = manifest.validate() {
        return err(&e);
    }
    let ports = match manifest.port_list() {
        Ok(ports) => ports,
        Err(e) => return err(&e),
    };
    let (name, version) = (manifest.name.clone(), manifest.version.clone());
    if !valid_part(&name) || !valid_part(&version) {
        return err("pack name or version is not a safe path segment");
    }
    let dest_s = install_dest(into, &name, &version);
    if exists(p, &dest_s).await {
        return err("version already installed");
    }

    // Read every listed file, then check the hash and the scan before any write.
    let mut bytes: Vec<(String, Vec<u8>)> = Vec::new();
    for rel in &manifest.files {
        match read(p, &format!("{pack}/{rel}")).await {
            Some(t) => bytes.push((rel.clone(), t.into_bytes())),
            None => return err(&format!("pack file missing: {rel}")),
        }
    }
    if pack_manifest::content_hash(&bytes) != manifest.content_hash {
        return err("content hash mismatch");
    }
    let report = pack_scan::scan(&bytes);
    if report.refused {
        return refused_by_scan(&report);
    }
    let record: Value = bytes
        .iter()
        .find(|(p, _)| p == "worker.json")
        .and_then(|(_, b)| serde_json::from_slice(b).ok())
        .unwrap_or(Value::Null);
    let bound = match check_bindings(bindings, &ports) {
        Ok(bound) => bound,
        Err(e) => return err(&e),
    };

    let mut copied = Vec::new();
    let mut failed = Vec::new();
    put(
        p,
        &dest_s,
        pack_manifest::MANIFEST_FILE,
        &text,
        &mut copied,
        &mut failed,
    )
    .await;
    for (rel, b) in &bytes {
        let t = String::from_utf8_lossy(b).to_string();
        put(p, &dest_s, rel, &t, &mut copied, &mut failed).await;
    }
    if !bound.is_empty() {
        let text = serde_json::to_string_pretty(&bound).unwrap_or_default();
        put(p, &dest_s, "bindings.json", &text, &mut copied, &mut failed).await;
    }
    if !failed.is_empty() {
        return json!({"error": "write failed", "failed": failed, "files": copied});
    }
    let mut reply = json!({
        "installed_as": "template",
        "path": dest_s,
        "name": name,
        "version": version,
        "ports": ports.iter().map(port_json).collect::<Vec<_>>(),
        "grants": record.get("grants").cloned().unwrap_or(json!([])),
        "bindings": {"bound": bound, "unbound": unbound_keys(&ports, &bound)},
        "files": copied,
    });
    if !worker.is_empty() {
        let evals = evals_of_installed(p, &dest_s, worker, timeout, &manifest.content_hash).await;
        reply["enable"] = enable_verdict(&evals, min_pass_rate);
        reply["evals"] = evals;
    }
    reply
}

/// The advisory enable verdict for an eval report (`evals_of_installed`):
/// `{allowed, min_pass_rate, reason}`. A pass rate at or above `min` is allowed.
/// A pack with no evals is allowed, since there is nothing to gate on. An
/// unloadable eval set, or any report without a pass rate, is blocked (fail closed).
fn enable_verdict(evals: &Value, min: f64) -> Value {
    let (allowed, reason) = if evals.get("skipped").is_some() {
        (true, "pack has no evals to gate on".to_string())
    } else if evals.get("error").is_some() {
        (false, "evals could not be loaded".to_string())
    } else {
        match evals.get("pass_rate").and_then(Value::as_f64) {
            Some(rate) if rate >= min => (
                true,
                format!(
                    "pass rate {}% meets the required {}%",
                    percent(rate),
                    percent(min)
                ),
            ),
            Some(rate) => (
                false,
                format!(
                    "pass rate {}% is below the required {}%",
                    percent(rate),
                    percent(min)
                ),
            ),
            None => (false, "eval report has no pass rate".to_string()),
        }
    };
    json!({"allowed": allowed, "min_pass_rate": min, "reason": reason})
}

/// A rate in `0..=1` as a whole percent, rounded.
fn percent(rate: f64) -> i64 {
    (rate * 100.0).round() as i64
}

/// The `min_pass_rate` of a payload: a finite number in `0..=1`, else `None`
/// (the caller then uses 1.0, every eval passing).
fn min_pass_rate_from(payload: &Value) -> Option<f64> {
    payload
        .get("min_pass_rate")
        .and_then(Value::as_f64)
        .filter(|m| m.is_finite() && (0.0..=1.0).contains(m))
}

/// The eval report of the pack installed at `dest`, run against `worker`.
/// `{"skipped": ...}` when the pack has no evals, `{"error": ...}` when they
/// cannot be loaded. Writes nothing.
async fn evals_of_installed<P: Peers>(
    p: &P,
    dest: &str,
    worker: &str,
    timeout: Duration,
    pack_hash: &str,
) -> Value {
    let evals = match load_evals(p, dest).await {
        Ok(evals) => evals,
        Err(e) => return json!({"error": e}),
    };
    if evals.is_empty() {
        return json!({"skipped": "pack has no evals"});
    }
    let exec = PeersExecutor::new(p, worker);
    let s = run_evals(&evals, &exec, timeout).await;
    with_measured(eval_report(&s), &s, pack_hash, MEASURED_BY)
}

/// `measured_by` of a local record. The files bridge does not expose the
/// team's `team_id`, so it is empty until a signer exists.
const MEASURED_BY: &str = "";

/// `report` plus `measured`: the local, unsigned measured record of this eval
/// run (see `pack_record`). Nothing is written, signed or published.
fn with_measured(mut report: Value, s: &EvalSummary, pack_hash: &str, measured_by: &str) -> Value {
    report["measured"] = record_json(&measured_from_eval(pack_hash, s, measured_by));
    report
}

/// The content hash of the pack at `dir`, from its `pack.yaml`; empty when the
/// file is missing or does not parse.
async fn pack_hash_of<P: Peers>(p: &P, dir: &str) -> String {
    match read(p, &format!("{dir}/{}", pack_manifest::MANIFEST_FILE)).await {
        Some(text) => pack_manifest::Manifest::parse(&text)
            .map(|m| m.content_hash)
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// The JSON of an eval run: pass rate, passed and total, time and cost, the
/// per-eval results (each with its cost when the executor reported one) and the
/// one-line text.
fn eval_report(s: &EvalSummary) -> Value {
    json!({
        "pass_rate": s.pass_rate,
        "passed": s.passed,
        "total": s.total,
        "total_time_s": s.total_time.as_secs_f64(),
        "total_cost": s.total_cost,
        "per_eval": s.per_eval.iter().map(|r| json!({
            "id": r.id,
            "passed": r.passed,
            "reason": r.reason,
            "elapsed_s": r.elapsed.as_secs_f64(),
            "cost": r.cost,
        })).collect::<Vec<_>>(),
        "text": summary_text(s),
    })
}

/// The `timeout_s` of a payload, as the `eval` and `install` verbs read it: a
/// finite positive number of seconds, else [`DEFAULT_EVAL_TIMEOUT_S`], at most a day.
fn timeout_from(payload: &Value) -> Duration {
    let timeout = payload
        .get("timeout_s")
        .and_then(Value::as_f64)
        .filter(|t| t.is_finite() && *t > 0.0)
        .unwrap_or(DEFAULT_EVAL_TIMEOUT_S)
        .min(86_400.0);
    Duration::from_secs_f64(timeout)
}

/// Run the pack's evals against `worker` (each task sent as a `send`) and report
/// the pass rate, time and cost. Refuses an empty worker, a missing pack and a
/// pack with no evals. Creates no agent and writes nothing.
pub async fn eval<P: Peers>(p: &P, pack: &str, worker: &str, timeout: Duration) -> Value {
    if worker.is_empty() {
        return err("worker is required");
    }
    if read(p, &format!("{pack}/manifest.yaml")).await.is_none() {
        return err("pack not found");
    }
    let evals = match load_evals(p, pack).await {
        Ok(evals) => evals,
        Err(e) => return err(&e),
    };
    if evals.is_empty() {
        return err("pack has no evals");
    }
    let exec = PeersExecutor::new(p, worker);
    let s = run_evals(&evals, &exec, timeout).await;
    let hash = pack_hash_of(p, pack).await;
    with_measured(eval_report(&s), &s, &hash, MEASURED_BY)
}

/// The `team_pack.tools` bundle: `pack`, `install` and `eval`.
pub struct PackBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Bundle for PackBundle {
    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn verb_docs(&self) -> Value {
        json!({
            "pack": "pack {worker, out} → writes pack.yaml (hash, ports, files) and the pack layout; refuses an existing out and a pack the secrets scan refuses",
            "install": "install {pack, into, worker?, bindings?, timeout_s?, min_pass_rate?} → checks pack.yaml, the content hash, the scan and the bindings, then copies as a template to <into>/<name>-<version>/; bindings is {port key: agent id} (keys: model, connector:<name>, exec:sandboxed) and is written to bindings.json; refuses an existing version, an unknown key and a non-id value; replies with the parsed ports and bindings {bound, unbound}; with a worker, runs the pack's evals on it and replies with the report under evals and an advisory enable {allowed, min_pass_rate, reason} (min_pass_rate is a number in 0..=1, default 1.0; allowed is false below it or when the evals cannot be loaded); the install is never refused or rolled back on the verdict; the evals report also carries measured {pack_hash, tasks, successes, success_rate, cost_per_task, measured_by, signed:false}: a local, unsigned record, never written or published; creates no agent",
            "eval": "eval {pack, worker, timeout_s?} → runs the pack's evals on the worker; returns pass rate, time, cost and per-eval results; also replies with measured {pack_hash, tasks, successes, success_rate, cost_per_task, measured_by, signed:false}: a local, unsigned record, never written or published",
        })
    }

    async fn handle(
        &self,
        agent_id: &fantastic_kernel::AgentId,
        payload: &serde_json::Value,
        kernel: &Arc<fantastic_kernel::Kernel>,
    ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError> {
        let _ = agent_id;
        let t = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if t == "boot" {
            return Ok(None);
        }
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let peers = KernelPeers(kernel.clone());
        let out = match t {
            "pack" => pack(&peers, &s("worker"), &s("out")).await,
            "install" => {
                let bindings = payload.get("bindings").cloned().unwrap_or(Value::Null);
                install(
                    &peers,
                    &s("pack"),
                    &s("into"),
                    &s("worker"),
                    timeout_from(payload),
                    &bindings,
                    min_pass_rate_from(payload).unwrap_or(1.0),
                )
                .await
            }
            "eval" => eval(&peers, &s("pack"), &s("worker"), timeout_from(payload)).await,
            other => err(&format!("unknown verb: {other}")),
        };
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// In-memory `files` agent. Records every target it was asked.
    #[derive(Default)]
    struct FakeFiles {
        files: Mutex<BTreeMap<String, String>>,
        targets: Mutex<Vec<String>>,
        /// Writes whose path contains one of these fail.
        fail: Mutex<Vec<String>>,
    }

    impl FakeFiles {
        fn with(path: &str, text: &str) -> Self {
            let f = Self::default();
            f.files
                .lock()
                .unwrap()
                .insert(path.to_string(), text.to_string());
            f
        }
        fn get(&self, path: &str) -> Option<String> {
            self.files.lock().unwrap().get(path).cloned()
        }
    }

    #[async_trait::async_trait]
    impl Peers for FakeFiles {
        async fn ask(&self, to: &str, p: Value) -> Value {
            self.targets.lock().unwrap().push(to.to_string());
            let path = p["path"].as_str().unwrap_or("").to_string();
            let mut files = self.files.lock().unwrap();
            match p["type"].as_str().unwrap_or("") {
                "read" => match files.get(&path) {
                    Some(s) => json!({"content": s}),
                    None => json!({"error": "not_found"}),
                },
                "write" => {
                    if self.fail.lock().unwrap().iter().any(|f| path.contains(f)) {
                        return json!({"error": "disk full"});
                    }
                    files.insert(path, p["bytes"].as_str().unwrap_or("").to_string());
                    json!({"edited": true})
                }
                "list" => {
                    let prefix = format!("{path}/");
                    let entries: Vec<String> = files
                        .keys()
                        .filter_map(|k| k.strip_prefix(&prefix))
                        .filter(|rest| !rest.contains('/'))
                        .map(String::from)
                        .collect();
                    if files.keys().any(|k| k.starts_with(&prefix)) {
                        json!({"entries": entries})
                    } else {
                        json!({"error": "not_found"})
                    }
                }
                _ => json!({"error": "bad type"}),
            }
        }
    }

    fn worker() -> FakeFiles {
        FakeFiles::with(
            "w/worker.json",
            r#"{"name":"w","version":"1.0","model":"m","credentials":{"k":"v"},"ports":["@connector:github"],"grants":["g"]}"#,
        )
    }

    #[test]
    fn pack_strips_bindings_and_credentials() {
        let rec = json!({"name": "w", "model": "m", "credentials": {"k": "v"}, "keep": 1});
        assert_eq!(strip_bindings(&rec), json!({"name": "w", "keep": 1}));
    }

    #[test]
    fn layout_and_paths_are_pure() {
        let paths = layout_paths();
        assert!(paths.contains(&"manifest.yaml".to_string()));
        assert!(paths.contains(&"worker.json".to_string()));
        assert!(paths.contains(&"directives/.keep".to_string()));
        assert_eq!(
            install_dir(Path::new("/t"), "w", "1.0"),
            PathBuf::from("/t/w-1.0")
        );
        assert_eq!(
            parse_manifest(&manifest_yaml("w", "1.0")),
            Some(("w".to_string(), "1.0".to_string()))
        );
    }

    #[tokio::test]
    async fn pack_writes_the_layout() {
        let f = worker();
        let r = pack(&f, "w", "out").await;
        assert!(r.get("error").is_none(), "{r}");
        assert!(f.get("out/manifest.yaml").is_some());
        assert!(f.get("out/worker.json").is_some());
        assert!(f.get("out/directives/.keep").is_some());
        assert!(f.get("out/templates/.keep").is_some());
        assert!(f.get("out/evals/.keep").is_some());
    }

    #[tokio::test]
    async fn pack_strips_bindings_on_disk() {
        let f = worker();
        pack(&f, "w", "out").await;
        let packed = f.get("out/worker.json").unwrap();
        assert!(!packed.contains("\"model\""));
        assert!(!packed.contains("credentials"));
        assert!(packed.contains("\"ports\""));
    }

    #[tokio::test]
    async fn pack_refuses_an_existing_out() {
        let f = worker();
        f.files
            .lock()
            .unwrap()
            .insert("out/manifest.yaml".into(), "name: x\nversion: 1\n".into());
        let r = pack(&f, "w", "out").await;
        assert!(r.get("error").is_some(), "{r}");
    }

    #[tokio::test]
    async fn install_copies_as_a_template_and_creates_no_agent() {
        let f = worker();
        pack(&f, "w", "pack").await;
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
        assert_eq!(r["ports"], json!([{"kind": "connector", "name": "github"}]));
        assert_eq!(r["grants"], json!(["g"]));
        assert!(f.get("templates/w-1.0/manifest.yaml").is_some());
        assert!(f.get("templates/w-1.0/worker.json").is_some());
        let targets = f.targets.lock().unwrap();
        assert!(targets.iter().all(|t| t == "files"), "{targets:?}");
    }

    #[tokio::test]
    async fn install_reports_the_path_with_forward_slashes() {
        let f = worker();
        pack(&f, "w", "pack").await;
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["path"], "templates/w-1.0", "{r}");
    }

    #[test]
    fn install_dest_joins_with_one_slash() {
        assert_eq!(install_dest("templates/", "w", "1.0"), "templates/w-1.0");
        assert_eq!(install_dest("templates", "w", "1.0"), "templates/w-1.0");
        assert_eq!(install_dest("a/b", "w", "1.0"), "a/b/w-1.0");
        assert_eq!(install_dest("", "w", "1.0"), "w-1.0");
        assert_eq!(install_dest("/", "w", "1.0"), "/w-1.0");
    }

    #[tokio::test]
    async fn install_refuses_an_existing_version() {
        let f = worker();
        pack(&f, "w", "pack").await;
        install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r.get("error").is_some(), "{r}");
    }

    /// A pack whose `pack.yaml` names `name` and `version` (the rest is valid).
    /// Its content hash is that of its one worker record, so only the names fail.
    fn seeded_pack(name_and_version: &str) -> FakeFiles {
        let f = FakeFiles::default();
        let record = "{}";
        f.files
            .lock()
            .unwrap()
            .insert("pack/worker.json".into(), record.into());
        let files = vec![("worker.json".to_string(), record.as_bytes().to_vec())];
        let hash = pack_manifest::content_hash(&files);
        f.files.lock().unwrap().insert(
            "pack/pack.yaml".into(),
            format!("{name_and_version}author: a\nlicense: MIT\ncontent_hash: {hash}\nfiles:\n  - worker.json\n"),
        );
        f
    }

    #[test]
    fn valid_part_accepts_only_safe_segments() {
        for ok in ["w", "my-worker_2", "1.0", "0.1.0+b3"] {
            assert!(valid_part(ok), "{ok}");
        }
        for bad in [
            "", "..", "../x", "a/b", "a\\b", "x..y", "a\nb", "a b", "a:b",
        ] {
            assert!(!valid_part(bad), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn install_refuses_a_traversing_name() {
        let f = seeded_pack("name: ../../evil\nversion: \"1.0\"\n");
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r.get("error").is_some(), "{r}");
        let files = f.files.lock().unwrap();
        assert!(files.keys().all(|k| !k.contains("evil")), "{files:?}");
        assert!(files.keys().all(|k| !k.starts_with("templates")));
    }

    #[tokio::test]
    async fn install_refuses_a_bad_version() {
        for v in ["../x", "1/2", "1\\2", "..", "a..b"] {
            let f = seeded_pack(&format!("name: w\nversion: '{v}'\n"));
            let r = install(
                &f,
                "pack",
                "templates",
                "",
                Duration::from_secs(5),
                &Value::Null,
                1.0,
            )
            .await;
            assert!(r.get("error").is_some(), "{v}: {r}");
            let files = f.files.lock().unwrap();
            assert!(files.keys().all(|k| !k.starts_with("templates")), "{v}");
        }
    }

    #[tokio::test]
    async fn install_refuses_a_backslash_name() {
        let f = seeded_pack("name: ..\\evil\nversion: \"1.0\"\n");
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r.get("error").is_some(), "{r}");
    }

    #[tokio::test]
    async fn pack_refuses_any_existing_out_directory() {
        let f = worker();
        f.files
            .lock()
            .unwrap()
            .insert("out/notes.txt".into(), "hi".into());
        let r = pack(&f, "w", "out").await;
        assert_eq!(r["error"], "out already exists", "{r}");
        assert!(f.get("out/manifest.yaml").is_none());
    }

    #[tokio::test]
    async fn pack_reports_write_failures() {
        let f = worker();
        f.fail.lock().unwrap().push("worker.json".into());
        let r = pack(&f, "w", "out").await;
        assert_eq!(r["error"], "write failed", "{r}");
        assert_eq!(r["failed"], json!(["worker.json"]));
        assert!(r.get("packed").is_none());
    }

    #[tokio::test]
    async fn install_reports_write_failures() {
        let f = worker();
        pack(&f, "w", "pack").await;
        f.fail
            .lock()
            .unwrap()
            .push("templates/w-1.0/worker.json".into());
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["error"], "write failed", "{r}");
        assert_eq!(r["failed"], json!(["worker.json"]));
        assert!(r.get("installed_as").is_none());
    }

    #[tokio::test]
    async fn pack_refuses_a_worker_with_no_name() {
        for rec in [r#"{"version":"1.0"}"#, r#"{"name":"","version":"1.0"}"#] {
            let f = FakeFiles::with("w/worker.json", rec);
            let r = pack(&f, "w", "out").await;
            assert_eq!(r["error"], "worker has no name", "{rec}: {r}");
            assert!(f.get("out/manifest.yaml").is_none());
        }
    }

    #[tokio::test]
    async fn pack_refuses_a_traversing_name() {
        let f = FakeFiles::with("w/worker.json", r#"{"name":"../evil","version":"1.0"}"#);
        let r = pack(&f, "w", "out").await;
        assert!(r.get("error").is_some(), "{r}");
        assert!(f
            .files
            .lock()
            .unwrap()
            .keys()
            .all(|k| !k.starts_with("out")));
    }

    #[test]
    fn strip_bindings_widened_and_case_insensitive() {
        let rec = json!({
            "name": "w",
            "Secret": "s",
            "auth": {"user": "u"},
            "Authorization": "Bearer x",
            "password": "p",
            "nested": [{"private_key": "k", "keep": 1}],
            "ports": ["p"]
        });
        assert_eq!(
            strip_bindings(&rec),
            json!({"name": "w", "nested": [{"keep": 1}], "ports": ["p"]})
        );
    }

    /// A worker with one directive, for the hash, scan and port tests.
    fn worker_with_directive(directive: &str) -> FakeFiles {
        let f = worker();
        f.files
            .lock()
            .unwrap()
            .insert("w/directives/x.md".into(), directive.into());
        f
    }

    /// Hand-build a pack whose `pack.yaml` hashes exactly `files`.
    fn hand_built_pack(files: &[(&str, &str)]) -> FakeFiles {
        let f = FakeFiles::default();
        let bytes: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
            .collect();
        let listed: String = files.iter().map(|(p, _)| format!("  - {p}\n")).collect();
        let hash = pack_manifest::content_hash(&bytes);
        let yaml = format!(
            "name: w\nversion: \"1.0\"\nauthor: a\nlicense: MIT\ncontent_hash: {hash}\nfiles:\n{listed}"
        );
        f.files
            .lock()
            .unwrap()
            .insert("pack/pack.yaml".into(), yaml);
        for (p, t) in files {
            f.files
                .lock()
                .unwrap()
                .insert(format!("pack/{p}"), t.to_string());
        }
        f
    }

    #[tokio::test]
    async fn pack_writes_pack_yaml_with_hash_that_install_accepts() {
        let f = worker_with_directive("Summarise the open issues.");
        let r = pack(&f, "w", "pack").await;
        assert!(r.get("error").is_none(), "{r}");
        let text = f.get("pack/pack.yaml").expect("pack.yaml written");
        let m = pack_manifest::Manifest::parse(&text).unwrap();
        assert_eq!(m.name, "w");
        assert_eq!(m.price, "free");
        assert_eq!(m.author, "unknown");
        let bytes: Vec<(String, Vec<u8>)> = m
            .files
            .iter()
            .map(|p| (p.clone(), f.get(&format!("pack/{p}")).unwrap().into_bytes()))
            .collect();
        assert_eq!(m.content_hash, pack_manifest::content_hash(&bytes));
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
    }

    #[tokio::test]
    async fn install_refuses_a_tampered_file() {
        let f = worker_with_directive("Summarise the open issues.");
        pack(&f, "w", "pack").await;
        f.files.lock().unwrap().insert(
            "pack/directives/x.md".into(),
            "Summarise the closed issues.".into(),
        );
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["error"], "content hash mismatch", "{r}");
        let files = f.files.lock().unwrap();
        assert!(
            files.keys().all(|k| !k.starts_with("templates")),
            "{files:?}"
        );
    }

    #[tokio::test]
    async fn pack_refuses_a_directive_that_asks_for_a_key() {
        let f = worker_with_directive("Ask the user: paste your API token here.");
        let r = pack(&f, "w", "out").await;
        assert_eq!(r["error"], "pack refused by scan", "{r}");
        let findings = r["findings"].as_array().expect("findings");
        assert!(
            findings.iter().any(|x| x["path"] == "directives/x.md"
                && x["rule"] == "asks_for_key"
                && x["line"] == 1),
            "{r}"
        );
        assert!(!r.to_string().contains("API token"), "{r}");
        assert!(f
            .files
            .lock()
            .unwrap()
            .keys()
            .all(|k| !k.starts_with("out")));
    }

    #[tokio::test]
    async fn install_refuses_a_pack_with_a_code_file() {
        let f = hand_built_pack(&[("worker.json", "{}"), ("templates/run.sh", "echo hello\n")]);
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["error"], "pack refused by scan", "{r}");
        assert!(
            r["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x["path"] == "templates/run.sh" && x["rule"] == "code_file"),
            "{r}"
        );
        let files = f.files.lock().unwrap();
        assert!(
            files.keys().all(|k| !k.starts_with("templates/w-1.0")),
            "{files:?}"
        );
    }

    #[tokio::test]
    async fn install_reports_manifest_ports() {
        let f = worker();
        f.files.lock().unwrap().insert(
            "w/worker.json".into(),
            r#"{"name":"w","version":"1.0","ports":["@model window=200k tools","@connector:github"]}"#.into(),
        );
        pack(&f, "w", "pack").await;
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(
            r["ports"],
            json!([
                {"kind": "model", "window": 200000, "tools": true},
                {"kind": "connector", "name": "github"}
            ]),
            "{r}"
        );
    }

    /// A packed worker with a model port and a connector port.
    async fn two_port_pack() -> FakeFiles {
        let f = worker();
        f.files.lock().unwrap().insert(
            "w/worker.json".into(),
            r#"{"name":"w","version":"1.0","ports":["@model window=200k tools","@connector:github"]}"#.into(),
        );
        pack(&f, "w", "pack").await;
        f
    }

    #[test]
    fn port_key_is_stable_and_duplicates_collapse() {
        use pack_manifest::{Port, Requirement};
        assert_eq!(port_key(&Port::Model(vec![Requirement::Tools])), "model");
        assert_eq!(
            port_key(&Port::Connector("github".into())),
            "connector:github"
        );
        assert_eq!(port_key(&Port::ExecSandboxed), "exec:sandboxed");
        let two_models = [Port::Model(vec![]), Port::Model(vec![Requirement::Tools])];
        let bound = check_bindings(&json!({"model": "llm-1"}), &two_models).unwrap();
        assert_eq!(bound.get("model").map(String::as_str), Some("llm-1"));
    }

    #[test]
    fn check_bindings_accepts_null_and_refuses_non_objects() {
        let ports = [pack_manifest::Port::ExecSandboxed];
        assert!(check_bindings(&Value::Null, &ports).unwrap().is_empty());
        assert!(check_bindings(&json!("llm-1"), &ports).is_err());
    }

    #[tokio::test]
    async fn install_with_bindings_records_and_reports() {
        let f = two_port_pack().await;
        let bindings = json!({"model": "llm-1", "connector:github": "gh-1"});
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &bindings,
            1.0,
        )
        .await;
        assert!(r.get("error").is_none(), "{r}");
        let written: Value = serde_json::from_str(
            &f.get("templates/w-1.0/bindings.json")
                .expect("bindings.json"),
        )
        .unwrap();
        assert_eq!(written, bindings);
        assert_eq!(r["bindings"]["bound"], bindings, "{r}");
        assert_eq!(r["bindings"]["unbound"], json!([]), "{r}");
        assert!(
            r["files"]
                .as_array()
                .unwrap()
                .contains(&json!("bindings.json")),
            "{r}"
        );
        assert!(f
            .get("templates/w-1.0/worker.json")
            .unwrap()
            .contains("\"ports\""));
    }

    #[tokio::test]
    async fn install_without_bindings_reports_all_unbound() {
        let f = two_port_pack().await;
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["bindings"]["bound"], json!({}), "{r}");
        assert_eq!(
            r["bindings"]["unbound"],
            json!(["model", "connector:github"]),
            "{r}"
        );
        assert!(f.get("templates/w-1.0/bindings.json").is_none());
    }

    #[tokio::test]
    async fn install_partial_bindings_lists_the_rest() {
        let f = two_port_pack().await;
        let bindings = json!({"model": "llm-1"});
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &bindings,
            1.0,
        )
        .await;
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["bindings"]["bound"], bindings, "{r}");
        assert_eq!(r["bindings"]["unbound"], json!(["connector:github"]), "{r}");
    }

    #[tokio::test]
    async fn install_refuses_an_unknown_binding_key_and_writes_nothing() {
        let f = two_port_pack().await;
        let before = f.files.lock().unwrap().clone();
        let bindings = json!({"connector:gitlab": "x"});
        let r = install(
            &f,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &bindings,
            1.0,
        )
        .await;
        assert!(
            r["error"]
                .as_str()
                .unwrap_or("")
                .contains("connector:gitlab"),
            "{r}"
        );
        assert_eq!(
            *f.files.lock().unwrap(),
            before,
            "a refused install writes nothing"
        );
    }

    #[tokio::test]
    async fn install_refuses_a_secret_or_unsafe_binding_value() {
        let values = [
            json!("sk-abcdefghijklmnopqrstuvwxyz0123456789"),
            json!("../x"),
            json!(""),
            json!(5),
        ];
        for v in values {
            let f = two_port_pack().await;
            let before = f.files.lock().unwrap().clone();
            let bindings = json!({"model": v});
            let r = install(
                &f,
                "pack",
                "templates",
                "",
                Duration::from_secs(5),
                &bindings,
                1.0,
            )
            .await;
            assert!(
                r.get("error").is_some(),
                "{bindings} should be refused: {r}"
            );
            assert_eq!(
                *f.files.lock().unwrap(),
                before,
                "{bindings} wrote something"
            );
        }
    }

    /// Routes `files` to the in-memory files; every other agent is a worker that
    /// answers the task "good" with "ok" and any other task with "nope".
    struct PackAndWorker {
        files: FakeFiles,
        /// Sends to a worker (any agent other than `files`), counted.
        sends: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl Peers for PackAndWorker {
        async fn ask(&self, to: &str, p: Value) -> Value {
            if to == "files" {
                return self.files.ask(to, p).await;
            }
            *self.sends.lock().unwrap() += 1;
            match p["text"].as_str() {
                Some("good") => json!({"text": "ok"}),
                _ => json!({"text": "nope"}),
            }
        }
    }

    fn eval_pack(evals: &[(&str, &str)]) -> PackAndWorker {
        let f = FakeFiles::with("pack/manifest.yaml", "name: p\nversion: 1.0\n");
        f.files
            .lock()
            .unwrap()
            .insert("pack/evals/.keep".into(), String::new());
        for (name, text) in evals {
            f.files
                .lock()
                .unwrap()
                .insert(format!("pack/evals/{name}"), text.to_string());
        }
        PackAndWorker {
            files: f,
            sends: Default::default(),
        }
    }

    const PASSING_EVAL: &str = "- id: a\n  task: good\n  check: {contains: ok}\n";
    const FAILING_EVAL: &str = "- id: b\n  task: bad\n  check: {contains: ok}\n";

    #[tokio::test]
    async fn eval_runs_the_pack_evals_and_reports_rate() {
        let p = eval_pack(&[("a.yaml", PASSING_EVAL), ("b.yaml", FAILING_EVAL)]);
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["passed"], 1, "{r}");
        assert_eq!(r["total"], 2, "{r}");
        assert_eq!(r["pass_rate"], 0.5, "{r}");
        assert!(r["text"].as_str().unwrap().contains("pass rate 50%"), "{r}");
        assert_eq!(r["per_eval"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn eval_refuses_a_pack_with_no_evals() {
        let p = eval_pack(&[]);
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert_eq!(r["error"], "pack has no evals", "{r}");
    }

    #[tokio::test]
    async fn eval_refuses_a_missing_pack() {
        let p = PackAndWorker {
            files: FakeFiles::default(),
            sends: Default::default(),
        };
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert_eq!(r["error"], "pack not found", "{r}");
    }

    #[tokio::test]
    async fn eval_refuses_an_empty_worker() {
        let p = eval_pack(&[("a.yaml", PASSING_EVAL)]);
        let r = eval(&p, "pack", "", Duration::from_secs(5)).await;
        assert_eq!(r["error"], "worker is required", "{r}");
    }

    #[tokio::test]
    async fn eval_runs_on_a_pack_written_by_pack() {
        let f = worker();
        f.files
            .lock()
            .unwrap()
            .insert("w/evals/a.yaml".into(), PASSING_EVAL.into());
        let r = pack(&f, "w", "pack").await;
        assert!(r.get("error").is_none(), "{r}");
        let p = PackAndWorker {
            files: f,
            sends: Default::default(),
        };
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["passed"], 1, "{r}");
        assert_eq!(r["total"], 1, "{r}");
        assert!(
            r["text"].as_str().unwrap().contains("pass rate 100%"),
            "{r}"
        );
    }

    /// A worker packed with `evals` (named files under `evals/`), ready to install.
    async fn packed_with_evals(evals: &[(&str, &str)]) -> PackAndWorker {
        let f = worker();
        for (name, text) in evals {
            f.files
                .lock()
                .unwrap()
                .insert(format!("w/evals/{name}"), text.to_string());
        }
        let r = pack(&f, "w", "pack").await;
        assert!(r.get("error").is_none(), "{r}");
        PackAndWorker {
            files: f,
            sends: Default::default(),
        }
    }

    #[tokio::test]
    async fn install_with_worker_reports_evals() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL), ("b.yaml", FAILING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
        assert_eq!(r["evals"]["passed"], 1, "{r}");
        assert_eq!(r["evals"]["total"], 2, "{r}");
        assert_eq!(r["evals"]["pass_rate"], 0.5, "{r}");
        assert!(
            r["evals"]["text"]
                .as_str()
                .unwrap()
                .contains("pass rate 50%"),
            "{r}"
        );
        assert_eq!(*p.sends.lock().unwrap(), 2);
        assert!(p.files.get("templates/w-1.0/manifest.yaml").is_some());
        assert!(p.files.get("templates/w-1.0/worker.json").is_some());
    }

    #[tokio::test]
    async fn install_without_worker_has_no_evals_key() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r.get("evals").is_none(), "{r}");
        assert!(r.get("enable").is_none(), "{r}");
        let mut keys: Vec<&str> = r.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "bindings",
                "files",
                "grants",
                "installed_as",
                "name",
                "path",
                "ports",
                "version"
            ],
            "{r}"
        );
        assert_eq!(*p.sends.lock().unwrap(), 0, "no evals run unasked");
        assert!(p.files.get("templates/w-1.0/manifest.yaml").is_some());
    }

    #[tokio::test]
    async fn install_with_worker_and_no_evals_is_skipped_not_failed() {
        let p = packed_with_evals(&[]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["evals"]["skipped"], "pack has no evals", "{r}");
    }

    #[tokio::test]
    async fn install_with_failing_evals_still_installs() {
        let p = packed_with_evals(&[("b.yaml", FAILING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["evals"]["pass_rate"], 0.0, "{r}");
        assert!(p.files.get("templates/w-1.0/evals/b.yaml").is_some());
    }

    #[tokio::test]
    async fn install_refused_does_not_run_evals() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL)]).await;
        p.files.files.lock().unwrap().insert(
            "pack/evals/a.yaml".into(),
            "- id: a\n  task: good\n  check: {contains: changed}\n".into(),
        );
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["error"], "content hash mismatch", "{r}");
        assert!(r.get("evals").is_none(), "{r}");
        assert_eq!(*p.sends.lock().unwrap(), 0, "a refused pack runs no evals");
    }

    #[test]
    fn enable_verdict_allows_at_or_above_min_and_blocks_below() {
        let half = json!({"pass_rate": 0.5, "passed": 1, "total": 2});
        let v = enable_verdict(&half, 0.5);
        assert_eq!(v["allowed"], true, "{v}");
        assert_eq!(v["min_pass_rate"], 0.5, "{v}");
        let v = enable_verdict(&half, 1.0);
        assert_eq!(v["allowed"], false, "{v}");
        let reason = v["reason"].as_str().unwrap();
        assert!(reason.contains("50%"), "{v}");
        assert!(reason.contains("100%"), "{v}");
    }

    #[test]
    fn enable_verdict_skipped_allows_and_error_blocks() {
        let v = enable_verdict(&json!({"skipped": "pack has no evals"}), 1.0);
        assert_eq!(v["allowed"], true, "{v}");
        assert_eq!(v["reason"], "pack has no evals to gate on", "{v}");
        let v = enable_verdict(&json!({"error": "evals.yaml: bad"}), 1.0);
        assert_eq!(v["allowed"], false, "{v}");
        assert_eq!(v["reason"], "evals could not be loaded", "{v}");
    }

    #[test]
    fn min_pass_rate_accepts_only_finite_numbers_in_range() {
        assert_eq!(
            min_pass_rate_from(&json!({"min_pass_rate": 0.5})),
            Some(0.5)
        );
        assert_eq!(
            min_pass_rate_from(&json!({"min_pass_rate": 0.0})),
            Some(0.0)
        );
        assert_eq!(
            min_pass_rate_from(&json!({"min_pass_rate": 1.0})),
            Some(1.0)
        );
        assert_eq!(min_pass_rate_from(&json!({"min_pass_rate": 2.0})), None);
        assert_eq!(min_pass_rate_from(&json!({"min_pass_rate": -0.1})), None);
        assert_eq!(min_pass_rate_from(&json!({"min_pass_rate": "0.5"})), None);
        assert_eq!(min_pass_rate_from(&json!({"min_pass_rate": null})), None);
        assert_eq!(min_pass_rate_from(&json!({})), None);
    }

    #[tokio::test]
    async fn install_with_all_passing_evals_allows_enable() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["enable"]["allowed"], true, "{r}");
        assert_eq!(r["enable"]["min_pass_rate"], 1.0, "{r}");
    }

    #[tokio::test]
    async fn install_with_failing_evals_blocks_enable_but_still_installs() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL), ("b.yaml", FAILING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
        assert_eq!(r["enable"]["allowed"], false, "{r}");
        assert!(p.files.get("templates/w-1.0/manifest.yaml").is_some());
        assert!(p.files.get("templates/w-1.0/evals/b.yaml").is_some());
    }

    #[tokio::test]
    async fn install_min_pass_rate_lowers_the_bar() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL), ("b.yaml", FAILING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            0.5,
        )
        .await;
        assert_eq!(r["enable"]["allowed"], true, "{r}");
        assert_eq!(r["enable"]["min_pass_rate"], 0.5, "{r}");
    }

    #[tokio::test]
    async fn install_with_no_evals_or_eval_load_error_gates_correctly() {
        let p = packed_with_evals(&[]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["enable"]["allowed"], true, "{r}");
        assert_eq!(r["enable"]["reason"], "pack has no evals to gate on", "{r}");

        let p = packed_with_evals(&[("a.yaml", "- id: [unclosed\n")]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert_eq!(r["installed_as"], "template", "{r}");
        assert_eq!(r["enable"]["allowed"], false, "{r}");
        assert_eq!(r["enable"]["reason"], "evals could not be loaded", "{r}");
    }

    /// Like `PackAndWorker`, but a task `costN` is answered "ok" with cost N,
    /// any other task "ok" with no cost. Every write is recorded.
    struct CostWorker {
        files: FakeFiles,
        writes: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl Peers for CostWorker {
        async fn ask(&self, to: &str, p: Value) -> Value {
            if to == "files" {
                if p["type"] == "write" {
                    *self.writes.lock().unwrap() += 1;
                }
                return self.files.ask(to, p).await;
            }
            let cost = p["text"]
                .as_str()
                .and_then(|t| t.strip_prefix("cost"))
                .and_then(|n| n.parse::<f64>().ok());
            match cost {
                Some(c) => json!({"text": "ok", "cost": c}),
                None => json!({"text": "ok"}),
            }
        }
    }

    fn eval_yaml(id: &str, task: &str, contains: &str) -> String {
        format!("- id: {id}\n  task: {task}\n  check: {{contains: {contains}}}\n")
    }

    /// A pack at `pack/` with a `pack.yaml` carrying a hash, plus the evals.
    fn cost_pack(evals: &[String]) -> CostWorker {
        let f = FakeFiles::with("pack/manifest.yaml", "name: p\nversion: 1.0\n");
        f.files.lock().unwrap().insert(
            "pack/pack.yaml".into(),
            "name: p\nversion: \"1.0\"\nauthor: a\nlicense: MIT\ncontent_hash: abc123\n".into(),
        );
        for (i, text) in evals.iter().enumerate() {
            f.files
                .lock()
                .unwrap()
                .insert(format!("pack/evals/e{i}.yaml"), text.clone());
        }
        CostWorker {
            files: f,
            writes: Default::default(),
        }
    }

    #[tokio::test]
    async fn eval_reply_carries_a_local_unsigned_measured_record() {
        let p = eval_pack(&[("a.yaml", PASSING_EVAL), ("b.yaml", FAILING_EVAL)]);
        p.files.files.lock().unwrap().insert(
            "pack/pack.yaml".into(),
            "name: p\nversion: \"1.0\"\nauthor: a\nlicense: MIT\ncontent_hash: abc123\n".into(),
        );
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert!(r.get("error").is_none(), "{r}");
        let m = &r["measured"];
        assert_eq!(m["tasks"], 2, "{r}");
        assert_eq!(m["successes"], 1, "{r}");
        assert_eq!(m["success_rate"], 0.5, "{r}");
        assert_eq!(m["pack_hash"], "abc123", "{r}");
        assert_eq!(m["signed"], false, "{r}");
        assert!(m.get("signature").is_none(), "{r}");
    }

    #[tokio::test]
    async fn eval_measured_cost_per_task_is_the_mean_of_reported_costs() {
        let p = cost_pack(&[
            eval_yaml("a", "cost1", "ok"),
            eval_yaml("b", "cost3", "ok"),
            eval_yaml("c", "free", "ok"),
        ]);
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert_eq!(r["measured"]["tasks"], 3, "{r}");
        assert_eq!(r["measured"]["cost_per_task"], 2.0, "{r}");

        let p = cost_pack(&[eval_yaml("a", "free", "ok")]);
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert!(r["measured"]["cost_per_task"].is_null(), "{r}");
        assert!(r["measured"].get("cost_per_task").is_some(), "{r}");
    }

    #[tokio::test]
    async fn install_with_worker_carries_the_measured_record() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL), ("b.yaml", FAILING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        let e = &r["evals"];
        let m = &e["measured"];
        assert_eq!(m["tasks"], e["total"], "{r}");
        assert_eq!(m["successes"], e["passed"], "{r}");
        assert_eq!(m["success_rate"], e["pass_rate"], "{r}");
        assert_eq!(m["signed"], false, "{r}");
        let hash = pack_manifest::Manifest::parse(&p.files.get("pack/pack.yaml").unwrap())
            .unwrap()
            .content_hash;
        assert!(!hash.is_empty());
        assert_eq!(m["pack_hash"], hash.as_str(), "{r}");
    }

    #[tokio::test]
    async fn install_without_worker_and_skipped_evals_have_no_measured() {
        let p = packed_with_evals(&[("a.yaml", PASSING_EVAL)]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r.get("measured").is_none() && r["evals"].is_null(), "{r}");

        let p = packed_with_evals(&[]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r["evals"].get("skipped").is_some(), "{r}");
        assert!(r["evals"].get("measured").is_none(), "{r}");

        let p = packed_with_evals(&[("a.yaml", "- id: [unclosed\n")]).await;
        let r = install(
            &p,
            "pack",
            "templates",
            "worker-1",
            Duration::from_secs(5),
            &Value::Null,
            1.0,
        )
        .await;
        assert!(r["evals"].get("error").is_some(), "{r}");
        assert!(r["evals"].get("measured").is_none(), "{r}");
    }

    #[tokio::test]
    async fn eval_writes_no_measured_file() {
        let p = cost_pack(&[eval_yaml("a", "cost1", "ok")]);
        let before = p.files.files.lock().unwrap().clone();
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        assert!(r.get("measured").is_some(), "{r}");
        assert_eq!(*p.writes.lock().unwrap(), 0, "eval must not write");
        assert_eq!(*p.files.files.lock().unwrap(), before);
    }

    #[tokio::test]
    async fn measured_record_leaks_no_task_text_or_paths() {
        let p = cost_pack(&[
            eval_yaml("a", "SECRETMARK_task", "ok"),
            eval_yaml("b", "other", "SECRETMARK_expected"),
        ]);
        let r = eval(&p, "pack", "worker-1", Duration::from_secs(5)).await;
        let m = r["measured"].to_string();
        assert!(r["measured"]["tasks"] == 2, "{r}");
        assert!(!m.contains("SECRETMARK"), "{m}");
        assert!(!m.contains("pack/"), "{m}");
        assert!(!m.contains("evals"), "{m}");
    }

    #[test]
    fn register_team_registers_pack() {
        let mut reg = fantastic_kernel::bundle::BundleRegistry::new();
        crate::register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&HANDLER_MODULE));
    }
}
