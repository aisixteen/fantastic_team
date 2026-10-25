use std::path::PathBuf;
use std::time::Duration;

use fantastic_team_app::{
    launch, start_args, start_setup, take_dir, target_from_args, Launched, Target,
};

/// How long the headless binary gets to answer 200 before we give up.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

fn fail(msg: &str) -> ! {
    eprintln!("fantastic-team-app: {msg}");
    std::process::exit(1);
}

/// The command line: `--dir` plus the target flags; a bad one exits with 2.
fn parse_args() -> (Option<PathBuf>, Target) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, args) = match take_dir(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("fantastic-team-app: {e}");
            std::process::exit(2);
        }
    };
    let target = match target_from_args(&args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("fantastic-team-app: {e}");
            std::process::exit(2);
        }
    };
    if let Target::Start { binary, port } = &target {
        println!(
            "fantastic-team-app: start {binary} {}",
            start_args(*port).join(" ")
        );
    }
    (dir, target)
}

/// Bring the target up: the headless binary, a tunnel, an attach, or the GitHub
/// setup flow (whose page URL the child prints).
async fn bring_up(dir: Option<PathBuf>, target: &Target) -> Result<Launched, String> {
    match target {
        Target::Setup { binary, name, org } => {
            start_setup(binary, name, org.as_deref(), READY_TIMEOUT).await
        }
        _ => launch(target, dir.as_deref(), READY_TIMEOUT).await,
    }
}

/// A setup child that ends normally has finished its flow: that is success.
fn ended(target: &Target, status: std::io::Result<std::process::ExitStatus>) -> ! {
    if matches!(target, Target::Setup { .. }) && status.as_ref().is_ok_and(|s| s.success()) {
        println!("fantastic-team-app: setup finished");
        std::process::exit(0);
    }
    fail(&format!("headless exited: {status:?}"));
}

#[cfg(not(feature = "tauri"))]
#[tokio::main]
async fn main() {
    let (dir, target) = parse_args();
    serve_and_wait(dir, target).await;
}

/// Without the window: print the URL and wait for Ctrl-C.
#[cfg(not(feature = "tauri"))]
async fn serve_and_wait(dir: Option<PathBuf>, target: Target) {
    let mut launched = match bring_up(dir, &target).await {
        Ok(l) => l,
        Err(e) => fail(&e),
    };
    println!("fantastic-team-app: web UI at {}", launched.url);
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        status = launched.exited() => {
            let _ = launched.stop().await;
            ended(&target, status);
        }
    }
    if let Err(e) = launched.stop().await {
        fail(&e);
    }
}

/// With the window: the runtime is shared with Tauri, the window runs on the
/// main thread, and closing it stops the child. A child that exits first ends
/// the app with code 1.
#[cfg(feature = "tauri")]
fn main() {
    use fantastic_team_app::window::{run_window, window_spec};

    let (dir, target) = parse_args();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => fail(&format!("the runtime: {e}")),
    };
    tauri::async_runtime::set(rt.handle().clone());
    let mut launched = match rt.block_on(bring_up(dir, &target)) {
        Ok(l) => l,
        Err(e) => fail(&e),
    };
    println!("fantastic-team-app: web UI at {}", launched.url);
    let spec = match window_spec(&launched.url) {
        Ok(s) => s,
        Err(e) => {
            let _ = rt.block_on(launched.stop());
            fail(&e);
        }
    };
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let ended_target = target.clone();
    let watcher = rt.spawn(async move {
        tokio::select! {
            _ = closed_rx => launched.stop().await,
            status = launched.exited() => {
                let _ = launched.stop().await;
                ended(&ended_target, status);
            }
        }
    });
    let handle = rt.handle().clone();
    let run = run_window(spec, move || {
        let _ = closed_tx.send(());
        match handle.block_on(watcher) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => fail(&e),
            Err(e) => fail(&format!("stop: {e}")),
        }
    });
    if let Err(e) = run {
        fail(&e);
    }
}
