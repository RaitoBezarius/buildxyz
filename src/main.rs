use cache::database::read_raw_buffer;
use fuser::spawn_mount2;
use log::info;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::process::{Command, Stdio};
use std::io;
use clap::Parser;
use memfile::MemFile;

mod fs;
mod cache;

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

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    cmd: String,
    #[arg(long = "db", default_value_os = cache::cache_dir())]
    database: PathBuf,
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();
    let mut stdout = io::stdout();

    // TODO: .expect should be replaced to catch errors and unmount filesystem no matter what.
    let terminate = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminate)).expect("Failed to set SIGTERM handler");
    stderrlog::new()
        //.module(module_path!())
        .verbosity(4)
        .init()
        .unwrap();
    let term = terminate.clone();
    ctrlc::set_handler(move || {
        term.store(true, Ordering::SeqCst);
    }).expect("Error setting the Ctrl-C handler");

    info!("Mounting the FUSE filesystem in the background...");

    // TODO: use tempdir for multiple instances
    let index_filename = args.database.join("files");
    let index_buffer = read_raw_buffer(index_filename).expect("Failed to read the index buffer!");
    let session = spawn_mount2(
        fs::BuildXYZ { index_buffer, ..Default::default() },
        "/tmp/buildxyz",
        &[]
    ).expect("Error spawning the FUSE filesystem in the background");

    info!("Running `{}`", args.cmd);

    // 1. Setup the environment variables
    // PATH_XXX="build env:negative lookup folder (FUSE)"

    // Let's keep PATH for now.
    let mut instrumented_env: HashMap<String, String> = std::env::vars()
        .filter(|&(ref k, _)|
            //             keep virtual envs.
            k == "PATH" || k == "PYTHONHOME"
        ).collect();

    instrumented_env.entry("PATH".to_string())
        .and_modify(|env_path| {
            *env_path = format!("/tmp/buildxyz/bin:{env_path}");
        });

    if let [cmd, cmd_args @ ..] = &args.cmd.split_ascii_whitespace().collect::<Vec<&str>>()[..] {
        let mut child_stdin_memfd = MemFile::create_default("child_stdin").expect("Failed to memfd_create");
        let mut child_stdout_memfd = MemFile::create_default("child_stdout").expect("Failed to memfd_create");

        let mut child = Command::new(cmd)
            //.stdin(child_stdin_memfd.try_clone().expect("Failed to dup memfd").into_file())
            //.stdout(child_stdout_memfd.try_clone().expect("Failed to dup memfd").into_file())
            .args(cmd_args)
            .env_clear()
            .envs(&instrumented_env)
            .spawn()
            .expect("Command failed to start");

        child.wait().expect("Failed to wait for child");
        info!("Command ended");

        while !terminate.load(Ordering::SeqCst) {}

        info!("Unmounting the filesystem...");
        session.join();
    } else {
        todo!("Dependent type theory in Rust");
    }

    Ok(())
}
