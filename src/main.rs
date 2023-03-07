use fuser::{spawn_mount2, Filesystem};
use log::info;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::process::Command;
use clap::Parser;
use nix_index::package;
use nix_index::database;

/// Request types between FUSE thread and UI thread
enum Request {
    /// An interactive search request for the given path to the UI thread
    InteractiveSearch(String)
}

/// Response types between UI thread and FUSE thread
enum Response {
    PackageSuggestion(String)
}

// 2 directories:
// - FUSE filesystem for negative lookups
// - normal filesystem for building the build environment (buildEnv)

struct BuildXYZ {
    db: database::Reader
}

impl Filesystem for BuildXYZ {
    fn init(&mut self, _req: &fuser::Request<'_>, _config: &mut fuser::KernelConfig) -> Result<(), i32> {
        Ok(())
    }

    fn destroy(&mut self) {
    }
}

fn cache_dir() -> &'static OsStr {
    let base = xdg::BaseDirectories::with_prefix("nix-index").unwrap();

    Box::leak(Box::new(base.get_cache_home())).as_os_str()
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    cmd: String,
    #[arg(long = "db", default_value_os = cache_dir())]
    database: PathBuf,
}

fn main() {
    let args = Args::parse();

    // TODO: .expect should be replaced to catch errors and unmount filesystem no matter what.
    let running = Arc::new(AtomicBool::new(true));
    stderrlog::new()
        //.module(module_path!())
        .verbosity(3)
        .init()
        .unwrap();
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting the Ctrl-C handler");

    info!("Mounting the FUSE filesystem in the background...");

    // TODO: use tempdir for multiple instances
    let index_filename = args.database.join("files");
    let session = spawn_mount2(
        BuildXYZ {
            db: database::Reader::open(&index_filename)
                .expect("Failed to open the nix-index DB")
        },
        "/tmp/buildxyz",
        &[]
    ).expect("Error spawning the FUSE filesystem in the background");

    info!("Running `{}`", args.cmd);

    // 1. Setup the environment variables
    // PATH_XXX="build env:negative lookup folder (FUSE)"

    // Let's keep PATH for now.
    let instrumented_env: HashMap<String, String> = std::env::vars()
        .filter(|&(ref k, _)|
            k == "PATH"
        ).collect();

    let mut child = Command::new(args.cmd)
        .env_clear()
        .envs(&instrumented_env)
        .spawn()
        .expect("Command failed to start");

    child.wait().expect("Failed to wait for child");
    info!("Command ended");
    while running.load(Ordering::SeqCst) {}

    info!("Unmounting the filesystem...");
    session.join();
}
