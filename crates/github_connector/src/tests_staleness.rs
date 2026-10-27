//! `staleness`: the facts for reusing a mark (commits and files since a recorded
//! commit, whether an issue changed), on the mock GitHub. Code gives facts only.

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::tests::{fx, mock_app, requests_to, token, Fx};
use crate::verbs::{issue_facts, summarize_compare};

const HEAD: &str = "9999999999999999999999999999999999999999";
const OLD: &str = "1111111111111111111111111111111111111111";

async fn mock_branch(gh: &MockServer, repo: &str, branch: &str, sha: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/{repo}/branches/{branch}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"name": branch, "commit": {"sha": sha}})),
        )
        .mount(gh)
        .await;
}

async fn mock_compare(gh: &MockServer, repo: &str, commit: &str, branch: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/{repo}/compare/{commit}...{branch}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(gh)
        .await;
}

fn commits(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| {
            json!({"sha": format!("{i:x}{}", "b".repeat(39)),
                   "commit": {"message": format!("subject {i}\n\nbody line")}})
        })
        .collect()
}

fn files(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({"filename": format!("src/f{i}.rs")}))
        .collect()
}

async fn widgets() -> Fx {
    let f = fx(json!({})).await;
    mock_app(&f.gh).await;
    mock_branch(&f.gh, "acme/widgets", "main", HEAD).await;
    f
}

fn ask(versions: Value) -> Value {
    json!({"type": "staleness", "versions": versions})
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_with_no_commit_returns_the_branch_head() {
    let f = widgets().await;
    let r = f
        .call(ask(json!([{"repo": "acme/widgets", "branch": "main"}])))
        .await;
    let e = &r["repos"][0];
    assert_eq!(e["repo"], "acme/widgets", "{r}");
    assert_eq!(e["branch"], "main", "{r}");
    assert_eq!(e["head"], HEAD, "{r}");
    assert_eq!(e["state"], "unknown", "{r}");
    assert_eq!(e["commits"], json!([]), "{r}");
    assert!(r.get("issue").is_none(), "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_lists_commits_and_files_since_the_commit() {
    let f = widgets().await;
    mock_compare(
        &f.gh,
        "acme/widgets",
        OLD,
        "main",
        json!({"status": "ahead", "ahead_by": 2, "behind_by": 0,
               "commits": commits(2), "files": files(3)}),
    )
    .await;
    let r = f
        .call(ask(
            json!([{"repo": "acme/widgets", "branch": "main", "commit": OLD}]),
        ))
        .await;
    let e = &r["repos"][0];
    assert_eq!(e["state"], "ahead", "{r}");
    assert_eq!(e["head"], HEAD, "{r}");
    assert_eq!(e["ahead_by"], 2, "{r}");
    assert_eq!(
        e["commits"],
        json!([
            {"sha": format!("0{}", "b".repeat(11)), "subject": "subject 0"},
            {"sha": format!("1{}", "b".repeat(11)), "subject": "subject 1"},
        ]),
        "{r}"
    );
    assert_eq!(
        e["files"],
        json!(["src/f0.rs", "src/f1.rs", "src/f2.rs"]),
        "{r}"
    );
    assert_eq!(e["truncated"], false, "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_same_commit_reports_same() {
    let f = widgets().await;
    mock_compare(
        &f.gh,
        "acme/widgets",
        HEAD,
        "main",
        json!({"status": "identical", "ahead_by": 0, "behind_by": 0, "commits": [], "files": []}),
    )
    .await;
    let r = f
        .call(ask(
            json!([{"repo": "acme/widgets", "branch": "main", "commit": HEAD}]),
        ))
        .await;
    let e = &r["repos"][0];
    assert_eq!(e["state"], "same", "{r}");
    assert_eq!(e["commits"], json!([]), "{r}");
    assert_eq!(e["files"], json!([]), "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_diverged_when_the_commit_is_not_an_ancestor() {
    let f = widgets().await;
    mock_compare(
        &f.gh,
        "acme/widgets",
        OLD,
        "main",
        json!({"status": "diverged", "ahead_by": 3, "behind_by": 4,
               "commits": commits(3), "files": files(1)}),
    )
    .await;
    let r = f
        .call(ask(
            json!([{"repo": "acme/widgets", "branch": "main", "commit": OLD}]),
        ))
        .await;
    let e = &r["repos"][0];
    assert_eq!(e["state"], "diverged", "{r}");
    assert_eq!(e["behind_by"], 4, "{r}");
    assert_eq!(e["ahead_by"], 3, "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_caps_commits_and_files_and_says_truncated() {
    let f = widgets().await;
    mock_compare(
        &f.gh,
        "acme/widgets",
        OLD,
        "main",
        json!({"status": "ahead", "ahead_by": 60, "behind_by": 0,
               "commits": commits(60), "files": files(130)}),
    )
    .await;
    let r = f
        .call(ask(
            json!([{"repo": "acme/widgets", "branch": "main", "commit": OLD}]),
        ))
        .await;
    let e = &r["repos"][0];
    assert_eq!(e["commits"].as_array().unwrap().len(), 50, "{r}");
    assert_eq!(e["files"].as_array().unwrap().len(), 100, "{r}");
    assert_eq!(e["truncated"], true, "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_issue_changed_compares_updated_at() {
    let f = fx(json!({})).await;
    mock_app(&f.gh).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/widgets/issues/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"number": 7, "title": "T", "updated_at": "2026-10-27T10:00:00Z"}),
        ))
        .mount(&f.gh)
        .await;
    let ask_issue = |sv: Option<&str>| {
        let mut issue = json!({"repo": "acme/widgets", "number": 7});
        if let Some(sv) = sv {
            issue["source_version"] = json!(sv);
        }
        json!({"type": "staleness", "issue": issue})
    };
    let same = f.call(ask_issue(Some("2026-10-27T10:00:00Z"))).await;
    assert_eq!(same["issue"]["changed"], false, "{same}");
    let diff = f.call(ask_issue(Some("2026-10-26T09:00:00Z"))).await;
    assert_eq!(diff["issue"]["changed"], true, "{diff}");
    let none = f.call(ask_issue(None)).await;
    assert_eq!(none["issue"]["changed"], Value::Null, "{none}");
    assert_eq!(
        none["issue"]["updated_at"], "2026-10-27T10:00:00Z",
        "{none}"
    );
    assert_eq!(none["issue"]["number"], 7, "{none}");
    assert!(none.get("repos").is_none(), "{none}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_checks_each_repo_in_one_call() {
    let f = fx(json!({"repos": ["acme/widgets", "acme/gadgets"]})).await;
    mock_app(&f.gh).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/gadgets/installation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 43})))
        .mount(&f.gh)
        .await;
    Mock::given(method("POST"))
        .and(path("/app/installations/43/access_tokens"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"token": token(), "expires_at": "2099-01-01T00:00:00Z"})),
        )
        .mount(&f.gh)
        .await;
    mock_branch(&f.gh, "acme/widgets", "main", HEAD).await;
    mock_branch(&f.gh, "acme/gadgets", "dev", OLD).await;
    let r = f
        .call(ask(json!([
            {"repo": "acme/gadgets", "branch": "dev"},
            {"repo": "acme/widgets", "branch": "main"}
        ])))
        .await;
    let repos = r["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 2, "{r}");
    assert_eq!(repos[0]["repo"], "acme/gadgets", "{r}");
    assert_eq!(repos[0]["head"], OLD, "{r}");
    assert_eq!(repos[1]["repo"], "acme/widgets", "{r}");
    assert_eq!(repos[1]["head"], HEAD, "{r}");
    assert_eq!(
        requests_to(&f.gh, "/app/installations/42/access_tokens")
            .await
            .len(),
        1
    );
    assert_eq!(
        requests_to(&f.gh, "/app/installations/43/access_tokens")
            .await
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_refuses_a_repo_off_the_allowlist_before_any_request() {
    let f = fx(json!({})).await;
    let r = f
        .call(ask(json!([
            {"repo": "acme/widgets", "branch": "main"},
            {"repo": "evil/other", "branch": "main"}
        ])))
        .await;
    assert_eq!(r["reason"], "bad_arg", "{r}");
    assert!(r["error"].as_str().unwrap().contains("evil/other"), "{r}");
    assert!(f.gh.received_requests().await.unwrap().is_empty());
    let issue = f
        .call(json!({"type": "staleness", "issue": {"repo": "evil/other", "number": 1}}))
        .await;
    assert!(
        issue["error"].as_str().unwrap().contains("evil/other"),
        "{issue}"
    );
    assert!(f.gh.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_needs_versions_or_issue() {
    let f = fx(json!({})).await;
    let r = f.call(json!({"type": "staleness"})).await;
    assert_eq!(r["reason"], "missing_arg", "{r}");
    let r = f.call(ask(json!([]))).await;
    assert_eq!(r["reason"], "missing_arg", "{r}");
    let r = f.call(ask(json!([{"repo": "acme/widgets"}]))).await;
    assert_eq!(r["reason"], "bad_arg", "{r}");
    assert!(r["error"].as_str().unwrap().contains("branch"), "{r}");
    let r = f
        .call(json!({"type": "staleness", "issue": {"repo": "acme/widgets"}}))
        .await;
    assert_eq!(r["reason"], "bad_arg", "{r}");
    assert!(r["error"].as_str().unwrap().contains("number"), "{r}");
    assert!(f.gh.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_api_error_is_named_without_the_token() {
    let f = widgets().await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/acme/widgets/compare/{OLD}...main")))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&f.gh)
        .await;
    let r = f
        .call(ask(
            json!([{"repo": "acme/widgets", "branch": "main", "commit": OLD}]),
        ))
        .await;
    assert_eq!(r["status"], 404, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("GitHub answered 404"),
        "{r}"
    );
    assert!(!r.to_string().contains(&token()), "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn staleness_is_logged_by_reflect_with_its_repo() {
    let f = widgets().await;
    let r = f
        .call(ask(json!([{"repo": "acme/widgets", "branch": "main"}])))
        .await;
    assert!(r.get("error").is_none(), "{r}");
    let rf = f.call(json!({"type": "reflect"})).await;
    assert_eq!(
        rf["recent_calls"],
        json!([{"verb": "staleness", "target": "acme/widgets", "result": "ok"}]),
        "{rf}"
    );
}

#[test]
fn summarize_compare_picks_subject_and_short_sha() {
    let cmp = json!({"status": "ahead", "ahead_by": 1, "behind_by": 0,
        "commits": [{"sha": "abcdef0123456789abcdef", "commit": {"message": "first line\nsecond"}}],
        "files": [{"filename": "a.rs"}]});
    let s = summarize_compare(&cmp, Some(OLD), HEAD);
    assert_eq!(
        s["commits"],
        json!([{"sha": "abcdef012345", "subject": "first line"}])
    );
    assert_eq!(s["files"], json!(["a.rs"]));
    assert_eq!(s["state"], "ahead");
    assert_eq!(s["truncated"], false);
    // no commit recorded: the head only, nothing to compare
    let none = summarize_compare(&Value::Null, None, HEAD);
    assert_eq!(none["state"], "unknown");
    assert_eq!(none["commits"], json!([]));
    // the same sha is the same, whatever the answer says
    assert_eq!(
        summarize_compare(&Value::Null, Some(HEAD), HEAD)["state"],
        "same"
    );
    // an old commit that is behind the branch (history rewritten) is diverged
    assert_eq!(
        summarize_compare(
            &json!({"status": "behind", "ahead_by": 0, "behind_by": 2}),
            Some(OLD),
            HEAD
        )["state"],
        "diverged"
    );
}

#[test]
fn issue_facts_null_when_no_source_version() {
    let issue = json!({"number": 7, "updated_at": "2026-10-27T10:00:00Z"});
    let none = issue_facts(&issue, None);
    assert_eq!(none["changed"], Value::Null);
    assert_eq!(none["updated_at"], "2026-10-27T10:00:00Z");
    assert_eq!(
        issue_facts(&issue, Some("2026-10-27T10:00:00Z"))["changed"],
        false
    );
    assert_eq!(issue_facts(&issue, Some("older"))["changed"], true);
}
