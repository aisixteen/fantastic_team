//! The ONE test boot over fantastic's `boot_kernel`: a folder is seeded, booted with the
//! team's bundles (and a test's own mock `Extra`s after them) and stopped again. No test
//! changes the process directory; `dir` is always an absolute temp folder the caller owns.
#![allow(dead_code)]

use fantastic_kernel::Kernel;
use fantastic_tui::Extra;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard, OnceCell};

/// What a folder is seeded with before it boots.
#[derive(Clone)]
pub enum Seed {
    /// fantastic's stock tree only.
    Stock,
    /// The stock tree and the team's blueprint (`fantastic_team::hydrate::seed`).
    Team,
    /// The stock tree and this blueprint text.
    Text(String),
}

/// A `web` host's state is process-wide, keyed by its agent id, so two kernels that hold a
/// `web` agent in one process undo each other. A booted folder with a `web` record holds
/// this from its boot until its stop.
static WEB_LOCK: Mutex<()> = Mutex::const_new(());

/// The user home store of this test process: empty, so no developer key mode or saved
/// set-up is read and no `credentials` store is made.
static HOME: OnceCell<tempfile::TempDir> = OnceCell::const_new();

/// Once per process, before its first boot: the home store, and one throw-away boot that
/// spends the process's one grant watch on a kernel nobody uses.
async fn setup() {
    HOME.get_or_init(|| async {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("FANTASTIC_HOME", home.path());
        let scratch = tempfile::tempdir().unwrap();
        fantastic_tui::seed_dir(scratch.path()).unwrap();
        let k = fantastic_tui::boot_kernel(scratch.path(), &[], None, None)
            .await
            .unwrap();
        fantastic_tui::stop_kernel(&k).await;
        home
    })
    .await;
}

/// A kernel booted by [`boot_at`]; it derefs to the kernel.
pub struct Booted {
    kernel: Arc<Kernel>,
    dir: PathBuf,
    seed: Seed,
    extras: Vec<Extra>,
    port: Option<u16>,
    web: Option<MutexGuard<'static, ()>>,
}

impl Deref for Booted {
    type Target = Arc<Kernel>;

    fn deref(&self) -> &Arc<Kernel> {
        &self.kernel
    }
}

impl Booted {
    /// A clone of the running kernel.
    pub fn kernel(&self) -> Arc<Kernel> {
        self.kernel.clone()
    }

    /// Stop the kernel, then release the web lock.
    pub async fn stop(self) {
        fantastic_tui::stop_kernel(&self.kernel).await;
    }

    /// Stop, then boot the same folder again with the same seed, extras and port.
    pub async fn restart(self) -> Booted {
        let (dir, seed, extras, port) = (
            self.dir.clone(),
            self.seed.clone(),
            self.extras.clone(),
            self.port,
        );
        self.stop().await;
        boot_at(&dir, seed, &extras, port).await
    }
}

/// Seed `dir` and boot it with `fantastic_team::register_team` and then `extras`. A seed
/// of a folder that already has `.fantastic` writes nothing. Panics with the error text
/// when the boot fails.
pub async fn boot_at(dir: &Path, seed: Seed, extras: &[Extra], web_port: Option<u16>) -> Booted {
    setup().await;
    match &seed {
        Seed::Stock => fantastic_tui::seed_dir(dir).map(|_| ()),
        Seed::Team => fantastic_team::hydrate::seed(dir).map(|_| ()),
        Seed::Text(t) => fantastic_tui::seed_dir_with(dir, &[t.as_str()]).map(|_| ()),
    }
    .unwrap_or_else(|e| panic!("seed {}: {e}", dir.display()));
    let web = if dir.join(".fantastic/agents/web/agent.yaml").exists() {
        Some(WEB_LOCK.lock().await)
    } else {
        None
    };
    let mut all: Vec<Extra> = vec![fantastic_team::register_team];
    all.extend_from_slice(extras);
    let kernel = fantastic_tui::boot_kernel(dir, &all, web_port, None)
        .await
        .unwrap_or_else(|e| panic!("boot {}: {e}", dir.display()));
    Booted {
        kernel,
        dir: dir.to_path_buf(),
        seed,
        extras: extras.to_vec(),
        port: web_port,
        web,
    }
}
