use fuser::{spawn_mount2, Filesystem};
use log::info;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

    let session = spawn_mount2(
        BuildXYZ {},
        "/tmp/buildxyz",
        &[]
    ).expect("Error spawning the FUSE filesystem in the background");

    info!("Running `{}`", args.cmd);
    while running.load(Ordering::SeqCst) {}

    info!("Unmounting the filesystem...");
    session.join();
}
