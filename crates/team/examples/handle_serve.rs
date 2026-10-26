//! A test driver for the e2e suite (`e2e/tests/app/handle_serves_port`), not product API.
//!
//! `handle_serve --dir D --port P` starts the team in D in this process through
//! `TeamHandle::start` with the web UI on P, prints `url: <url>`, waits for end of
//! stdin or SIGTERM, then stops the handle and exits 0. Nothing is spawned.

use std::path::PathBuf;

use fantastic_team::app_control::{TeamHandle, TeamOptions};
use tokio::io::AsyncReadExt;
use tokio::signal::unix::{signal, SignalKind};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main]
async fn main() {
    let Some(dir) = arg("--dir").map(PathBuf::from) else {
        eprintln!("usage: handle_serve --dir DIR [--port PORT]");
        std::process::exit(2);
    };
    let port = arg("--port").map(|p| p.parse::<u16>().expect("--port is a number"));
    let mut handle = match TeamHandle::start(&dir, TeamOptions { port }).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("handle_serve: {e}");
            std::process::exit(1);
        }
    };
    println!("url: {}", handle.url().unwrap_or_default());
    let mut term = signal(SignalKind::terminate()).expect("sigterm");
    let mut sink = Vec::new();
    let mut stdin = tokio::io::stdin();
    tokio::select! {
        _ = stdin.read_to_end(&mut sink) => {}
        _ = term.recv() => {}
    }
    if let Err(e) = handle.stop().await {
        eprintln!("handle_serve: stop failed: {e}");
        std::process::exit(1);
    }
}
