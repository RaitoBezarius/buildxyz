use fuser::{spawn_mount2, Filesystem};
use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::process::Command;
use clap::Parser;

// 2 directories:
// - FUSE filesystem for negative lookups
// - normal filesystem for building the build environment (buildEnv)

struct BuildXYZ {}

impl Filesystem for BuildXYZ {
    fn init(&mut self, _req: &fuser::Request<'_>, _config: &mut fuser::KernelConfig) -> Result<(), i32> {
        Ok(())
    }

    fn destroy(&mut self) {
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    cmd: String
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
    let session = spawn_mount2(
        BuildXYZ {},
        "/tmp/buildxyz",
        &[]
    ).expect("Error spawning the FUSE filesystem in the background");

    info!("Running `{}`", args.cmd);

    // 1. Setup the environment variables
    // PATH_XXX="build env:negative lookup folder (FUSE)"

    let instrumented_env: HashMap<String, String> = HashMap::new();
    Command::new(args.cmd)
        .env_clear()
        .envs(&instrumented_env)
        .spawn()
        .expect("Command failed to start");

    while running.load(Ordering::SeqCst) {}

    info!("Unmounting the filesystem...");
    session.join();
}
