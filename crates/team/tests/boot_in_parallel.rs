//! Proof: two folders boot with `boot_kernel` in parallel in one process, with no
//! change of the process directory, and each kernel sees only its own folder.

use fantastic_kernel::{AgentId, Item, Kernel};
use fantastic_tui::{boot_kernel, seed_dir, stop_kernel};
use serde_json::{json, Value};
use std::sync::Arc;

async fn list_root(k: &Arc<Kernel>) -> String {
    let reply = k
        .send(vec![Item::new(
            AgentId::from("files"),
            json!({"type": "list", "path": "."}),
        )])
        .await
        .into_iter()
        .next()
        .map(|o| o.result)
        .unwrap_or(Value::Null);
    reply.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_folders_boot_in_parallel_in_one_process() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("FANTASTIC_HOME", home.path());
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for d in [a.path(), b.path()] {
        seed_dir(d).unwrap();
    }
    std::fs::write(a.path().join("marker_a.txt"), "a").unwrap();
    std::fs::write(b.path().join("marker_b.txt"), "b").unwrap();

    let cwd = std::env::current_dir().unwrap();
    let (ka, kb) = tokio::join!(
        boot_kernel(a.path(), &[], None, None),
        boot_kernel(b.path(), &[], None, None)
    );
    let ka = ka.expect("boot A");
    let kb = kb.expect("boot B");

    let (wa, wb) = (ka.work_dir().unwrap(), kb.work_dir().unwrap());
    assert_ne!(wa, wb);
    assert_eq!(wa, a.path());
    assert_eq!(wb, b.path());
    let la = list_root(&ka).await;
    let lb = list_root(&kb).await;
    assert!(la.contains("marker_a.txt"), "A: {la}");
    assert!(!la.contains("marker_b.txt"), "A: {la}");
    assert!(lb.contains("marker_b.txt"), "B: {lb}");
    assert!(!lb.contains("marker_a.txt"), "B: {lb}");

    tokio::join!(stop_kernel(&ka), stop_kernel(&kb));

    let after = std::env::current_dir().unwrap();
    assert_eq!(cwd, after);
    assert_ne!(after, a.path());
    assert_ne!(after, b.path());
    for d in [a.path(), b.path()] {
        assert!(!d.join(".fantastic/lock.yaml").exists());
    }
}
