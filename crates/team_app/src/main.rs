use std::time::Duration;

use fantastic_team_app::{launch, start_args, take_dir, target_from_args, Target};

/// How long the headless binary gets to answer 200 before we give up.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

fn fail(msg: &str) -> ! {
    eprintln!("fantastic-team-app: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
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
    let mut launched = match launch(&target, dir.as_deref(), READY_TIMEOUT).await {
        Ok(l) => l,
        Err(e) => fail(&e),
    };
    println!("fantastic-team-app: web UI at {}", launched.url);
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        status = launched.exited() => {
            let _ = launched.stop().await;
            fail(&format!("headless exited: {status:?}"));
        }
    }
    if let Err(e) = launched.stop().await {
        fail(&e);
    }
}
