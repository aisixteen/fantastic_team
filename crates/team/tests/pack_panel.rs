//! The pack browser panel's page: it speaks only the verbs the pack agents
//! answer, previews before it approves and draws values as text.

use fantastic_team::pack_panel::HTML;

/// The text of one top-level JS function: from its header to the closing
/// brace at column zero. Line endings are normalised first (a Windows checkout
/// has CRLF).
fn js_fn(name: &str) -> String {
    let html = HTML.replace("\r\n", "\n");
    let start = html
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("no function {name}"));
    let end = html[start..].find("\n}\n").expect("closing brace") + start;
    html[start..end].to_string()
}

#[test]
fn panel_lists_packs_through_pack_listing_list() {
    assert!(HTML.contains("fantastic.send("));
    let load = js_fn("loadList");
    assert!(load.contains("ask('pack_listing'"), "{load}");
    assert!(load.contains("type: 'list'"), "{load}");
    assert!(load.contains("query"), "{load}");
}

#[test]
fn panel_shows_the_permission_preview_from_show() {
    let show = js_fn("showPack");
    assert!(
        show.contains("ask('pack_listing'") && show.contains("type: 'show'"),
        "{show}"
    );
    let draw = js_fn("drawDetail");
    for field in [
        "r.ports",
        "r.grants",
        "r.price",
        "r.install_decision",
        "r.evals_count",
        "r.measured",
        "r.installed",
        "r.revoked",
    ] {
        assert!(draw.contains(field), "drawDetail does not read {field}");
    }
}

#[test]
fn panel_runs_install_and_eval_through_the_same_verbs() {
    let payload = js_fn("installPayload");
    for key in [
        "type: 'install'",
        "pack:",
        "into:",
        "bindings:",
        "parent_id",
        "approve",
    ] {
        assert!(payload.contains(key), "install payload lacks {key}");
    }
    assert!(js_fn("previewInstall").contains("ask('pack', installPayload(false))"));
    assert!(js_fn("runInstall").contains("ask('pack', installPayload(true))"));
    let eval = js_fn("runEval");
    assert!(
        eval.contains("ask('pack'") && eval.contains("type: 'eval'"),
        "{eval}"
    );
    assert!(eval.contains("worker"), "{eval}");
    // every verb the page sends is one of the four the agents already answer
    let mut verbs: Vec<&str> = HTML
        .split("type: '")
        .skip(1)
        .filter_map(|rest| rest.split('\'').next())
        .collect();
    verbs.sort();
    verbs.dedup();
    assert_eq!(verbs, ["eval", "install", "list", "show"]);
    // and only to the two pack agents
    let mut targets: Vec<&str> = HTML
        .split("ask('")
        .skip(1)
        .filter_map(|rest| rest.split('\'').next())
        .collect();
    targets.sort();
    targets.dedup();
    assert_eq!(targets, ["pack", "pack_listing"]);
}

#[test]
fn panel_previews_before_approving() {
    let run = js_fn("runInstall");
    let confirm = run.find("window.confirm(").expect("asks first");
    let send = run.find("installPayload(true)").expect("sends approve");
    assert!(confirm < send, "{run}");
    // the preview never carries approve; only the approving payload does
    let payload = js_fn("installPayload");
    assert!(
        payload.contains("if (approve) payload.approve = true;"),
        "{payload}"
    );
    assert!(
        !js_fn("previewInstall").contains("true)"),
        "preview must not approve"
    );
}

#[test]
fn panel_asks_one_binding_per_unbound_port() {
    let render = js_fn("renderPreview");
    assert!(render.contains("bindings.unbound"), "{render}");
    assert!(render.contains("for (const key of unbound)"), "{render}");
    assert!(render.contains("renderBindings(unbound)"), "{render}");
    let binds = js_fn("renderBindings");
    assert!(binds.contains("forEach"), "{binds}");
    assert!(
        binds.contains("createElement") || binds.contains("el('input')"),
        "{binds}"
    );
    assert!(js_fn("collectBindings").contains("typed"));
}

#[test]
fn panel_draws_text_only() {
    for bad in [
        "innerHTML",
        "insertAdjacentHTML",
        "outerHTML",
        "document.write",
    ] {
        assert!(!HTML.contains(bad), "the panel uses {bad}");
    }
    assert!(js_fn("el").contains("textContent"));
}

#[test]
fn panel_install_result_draws_the_eval_pass_rate_time_and_cost() {
    // An install with a worker replies `evals` (the eval verb's report: pass_rate,
    // total_time_s, total_cost and the one-line `text` that names all three).
    let preview = js_fn("renderPreview");
    assert!(preview.contains("r.evals"), "{preview}");
    assert!(preview.contains("evals.text"), "{preview}");
    assert!(preview.contains("evals.pass_rate"), "{preview}");
    // As text only, through el(), never as markup.
    assert!(!preview.contains("innerHTML"), "{preview}");
}

#[test]
fn panel_eval_and_install_both_draw_the_eval_summary_text() {
    assert!(js_fn("runEval").contains("r.text"));
    let preview = js_fn("renderPreview");
    assert!(
        preview.contains("r.evals") && preview.contains("evals.text"),
        "{preview}"
    );
}
