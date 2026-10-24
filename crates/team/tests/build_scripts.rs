//! Runs `tests/build_github.sh` (the GitHub build script's failure and success cases).

#[cfg(unix)]
#[test]
fn build_github_fails_on_a_bad_patch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new("sh")
        .arg(root.join("tests").join("build_github.sh"))
        .output()
        .expect("run sh tests/build_github.sh");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("a_bad_patch_fails_with_a_message"), "{text}");
}
