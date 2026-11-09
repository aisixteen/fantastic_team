//! The credential mode is which fields the ref holds: `token` (personal) or
//! `app_id` + `private_key` (app). No credential means local only.

use fantastic_kernel::bootstrap;
use fantastic_kernel::Item;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::app::LOCAL_ONLY;
use crate::tests::{
    basic_of, decode, forge, fx, fx_fields, mock_app, pat, requests_to, setup, test_key, Fx,
    APP_ID, REF, STORE,
};

/// A fixture whose github agent reads its credential from an env-backed store.
async fn fx_env(env: Value) -> Fx {
    let f = fx(json!({})).await;
    let made =
        f.k.send(vec![Item::new(
            "kernel",
            json!({
                "type": "create_agent",
                "parent_id": bootstrap::DEFAULT_ROOT_ID,
                "id": "envcreds",
                "handler_module": fantastic_credential_store::HANDLER_MODULE,
                "backend": "env",
                "refs": {REF: {"host": f.gh.uri(), "env": env}},
                "callable_by": {"$self": "*", "github": "*"},
            }),
        )])
        .await
        .remove(0)
        .result;
    assert_eq!(made["id"], "envcreds", "{made}");
    let upd =
        f.k.send(vec![Item::new(
            "kernel",
            json!({
                "type": "update_agent", "id": "github",
                "app": {"credentials": {"store": "envcreds", "ref": REF}},
                "can_call": {STORE: "*", "envcreds": "*"},
            }),
        )])
        .await
        .remove(0)
        .result;
    assert!(upd.get("error").is_none(), "{upd}");
    f
}

async fn mock_get(gh: &MockServer, p: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(gh)
        .await;
}

async fn auths(gh: &MockServer) -> Vec<String> {
    gh.received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| {
            r.headers
                .get("authorization")
                .and_then(|h| h.to_str().ok())
                .map(str::to_string)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn personal_token_is_used_as_is_with_no_mint() {
    let f = fx_fields(json!({}), json!({"token": pat()})).await;
    mock_get(&f.gh, "/repos/acme/widgets/issues", json!([])).await;
    mock_get(
        &f.gh,
        "/repos/acme/widgets",
        json!({"default_branch": "main"}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/widgets/pulls"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"number": 9, "html_url": "https://x/pull/9", "draft": true})),
        )
        .mount(&f.gh)
        .await;
    let a = f
        .call(json!({"type": "issues", "repo": "acme/widgets"}))
        .await;
    let b = f
        .call(
            json!({"type": "pr_create", "repo": "acme/widgets", "branch": "work/x",
                     "title": "T", "body": "B"}),
        )
        .await;
    for r in [&a, &b] {
        assert!(r.get("error").is_none(), "{r}");
        assert!(!r.to_string().contains(&pat()), "{r}");
    }
    let want = format!("Bearer {}", pat());
    let seen = auths(&f.gh).await;
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|h| *h == want), "{seen:?}");
    assert!(requests_to(&f.gh, "/repos/acme/widgets/installation")
        .await
        .is_empty());
    assert!(requests_to(&f.gh, "/app/installations/42/access_tokens")
        .await
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn personal_push_sends_the_token_to_git_only() {
    let f = fx_fields(json!({}), json!({"token": pat()})).await;
    let forge = forge().await;
    setup(&f, &format!("{}/acme/widgets.git", forge.uri())).await;
    let r = f
        .call(json!({"type": "push", "path": "w", "branch": "work/gh"}))
        .await;
    assert!(r.get("error").is_some(), "{r}");
    let want = basic_of(&pat());
    let seen = forge.received_requests().await.unwrap();
    assert!(
        seen.iter().any(|q| {
            q.headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(want.as_str())
        }),
        "the forge never saw the personal token"
    );
    assert!(!r.to_string().contains(&pat()), "{r}");
    assert!(requests_to(&f.gh, "/repos/acme/widgets/installation")
        .await
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn whoami_in_personal_mode_reads_the_user() {
    let f = fx_fields(json!({}), json!({"token": pat()})).await;
    mock_get(&f.gh, "/user", json!({"login": "me", "id": 7})).await;
    let r = f.call(json!({"type": "whoami"})).await;
    assert_eq!(r["user"]["login"], "me", "{r}");
    assert_eq!(auths(&f.gh).await, vec![format!("Bearer {}", pat())]);
    assert!(requests_to(&f.gh, "/app").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn app_mode_reads_the_key_from_a_pem_file_path() {
    let f =
        fx_env(json!({"app_id": "TEAM_GH_T303_ID_F", "private_key": "TEAM_GH_T303_KEY_F"})).await;
    let pem_path = f.tmp.path().join("app.pem");
    std::fs::write(&pem_path, &test_key().0).unwrap();
    std::env::set_var("TEAM_GH_T303_ID_F", APP_ID);
    std::env::set_var("TEAM_GH_T303_KEY_F", &pem_path);
    mock_app(&f.gh).await;
    mock_get(&f.gh, "/repos/acme/widgets/issues", json!([])).await;
    let r = f
        .call(json!({"type": "issues", "repo": "acme/widgets"}))
        .await;
    assert!(r.get("error").is_none(), "{r}");
    let inst = requests_to(&f.gh, "/repos/acme/widgets/installation").await;
    assert_eq!(inst.len(), 1);
    let h = inst[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(decode(h.strip_prefix("Bearer ").unwrap()).iss, APP_ID);
    let text = r.to_string();
    assert!(!text.contains("app.pem") && !text.contains("BEGIN"), "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn app_mode_from_env_pem_text_mints_per_call() {
    let f =
        fx_env(json!({"app_id": "TEAM_GH_T303_ID_T", "private_key": "TEAM_GH_T303_KEY_T"})).await;
    std::env::set_var("TEAM_GH_T303_ID_T", APP_ID);
    std::env::set_var("TEAM_GH_T303_KEY_T", &test_key().0);
    mock_app(&f.gh).await;
    mock_get(&f.gh, "/repos/acme/widgets/issues", json!([])).await;
    for _ in 0..2 {
        let r = f
            .call(json!({"type": "issues", "repo": "acme/widgets"}))
            .await;
        assert!(r.get("error").is_none(), "{r}");
    }
    assert_eq!(
        requests_to(&f.gh, "/app/installations/42/access_tokens")
            .await
            .len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_credential_means_local_only_for_every_verb() {
    const VAR: &str = "TEAM_GH_T303_UNSET";
    std::env::remove_var(VAR);
    let f = fx_env(json!({"token": VAR})).await;
    let forge = forge().await;
    setup(&f, &format!("{}/acme/widgets.git", forge.uri())).await;
    let calls = vec![
        json!({"type": "issues", "repo": "acme/widgets"}),
        json!({"type": "issue_view", "repo": "acme/widgets", "number": 3}),
        json!({"type": "pr_create", "repo": "acme/widgets", "branch": "work/x", "title": "T"}),
        json!({"type": "comment", "repo": "acme/widgets", "number": 3, "body": "hi"}),
        json!({"type": "label", "repo": "acme/widgets", "number": 3, "add": ["a"]}),
        json!({"type": "pr_status", "repo": "acme/widgets", "number": 5}),
        json!({"type": "merge", "repo": "acme/widgets", "number": 7}),
        json!({"type": "issue_create", "repo": "acme/widgets", "title": "T", "body": "B"}),
        json!({"type": "staleness", "versions": [{"repo": "acme/widgets", "branch": "main"}]}),
        json!({"type": "whoami"}),
        json!({"type": "push", "path": "w", "branch": "work/gh"}),
    ];
    for c in calls {
        let r = f.call(c.clone()).await;
        assert_eq!(r["error"], LOCAL_ONLY, "{c}: {r}");
        assert_eq!(r["reason"], "not_found", "{c}: {r}");
    }
    assert!(f.gh.received_requests().await.unwrap().is_empty());
    assert!(forge.received_requests().await.unwrap().is_empty());

    // Setting the credential later enables GitHub; the records are unchanged.
    std::env::set_var(VAR, pat());
    mock_get(&f.gh, "/repos/acme/widgets/issues", json!([])).await;
    let r = f
        .call(json!({"type": "issues", "repo": "acme/widgets"}))
        .await;
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(auths(&f.gh).await, vec![format!("Bearer {}", pat())]);
    std::env::remove_var(VAR);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wrong_mode_s_missing_field_is_refused_by_name() {
    let pem = test_key().0.clone();
    let cases = [
        (
            json!({"app_id": APP_ID}),
            "has `app_id` but no `private_key`",
        ),
        (
            json!({"private_key": pem}),
            "has `private_key` but no `app_id`",
        ),
        (
            json!({"token": pat(), "app_id": APP_ID}),
            "holds `token` and app fields",
        ),
        (json!({"other": "x"}), "has no `token`"),
    ];
    for (fields, want) in cases {
        let f = fx_fields(json!({}), fields).await;
        let r = f
            .call(json!({"type": "issues", "repo": "acme/widgets"}))
            .await;
        let e = r["error"].as_str().unwrap_or_default();
        assert!(
            e.starts_with("team_github: credential `github/app` in `creds`:"),
            "{r}"
        );
        assert!(e.contains(want), "{want}: {r}");
        let text = r.to_string();
        assert!(!text.contains(&pat()) && !text.contains("BEGIN"), "{r}");
        assert!(f.gh.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reflect_shows_the_auth_mode() {
    const VAR: &str = "TEAM_GH_T303_UNSET_R";
    std::env::remove_var(VAR);
    let app = fx(json!({})).await;
    let personal = fx_fields(json!({}), json!({"token": pat()})).await;
    let local = fx_env(json!({"token": VAR})).await;
    let bad = fx_fields(json!({}), json!({"app_id": APP_ID})).await;
    let mut seen = Vec::new();
    for (f, want) in [
        (&app, json!({"mode": "app"})),
        (&personal, json!({"mode": "personal"})),
        (&local, json!({"mode": "local", "note": LOCAL_ONLY})),
    ] {
        let r = f.call(json!({"type": "reflect"})).await;
        assert_eq!(r["auth"], want, "{r}");
        seen.push(r.to_string());
    }
    let r = bad.call(json!({"type": "reflect"})).await;
    assert_eq!(r["auth"]["mode"], "invalid", "{r}");
    seen.push(r.to_string());
    for s in seen {
        assert!(!s.contains(&pat()) && !s.contains("BEGIN"), "{s}");
        assert!(!s.contains("eyJ"), "{s}");
    }
}
