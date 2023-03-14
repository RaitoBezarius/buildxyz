use cache::database::read_raw_buffer;
use clap::Parser;
use fuser::spawn_mount2;
use log::info;
use ::nix::sys::signal::Signal::{SIGTERM, SIGINT, SIGKILL};
use ::nix::unistd::Pid;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// mod instrument;
mod cache;
mod fs;
mod nix;
mod popcount;
mod runner;

enum EventMessage {
    Stop,
    Done
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
    /// In case of failures, retry automatically the invocation
    #[arg(long = "r", default_value_t = false)]
    retry: bool
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();

    // Signal to stop the current program
    // If sent twice, uses SIGKILL
    let (send_event, recv_event) = channel::<EventMessage>();
    let mut stop_count = 0;

    let ctrlc_event = send_event.clone();
    ctrlc::set_handler(move || {
        info!("Ctrl-C received...");
        ctrlc_event.send(EventMessage::Stop).expect("Failed to send Ctrl-C event to the main thread");
    }).expect("Failed to set Ctrl-C handler");
    // FIXME: register SIGTERM too.

    stderrlog::new()
        //.module(module_path!())
        .verbosity(4)
        .init()
        .unwrap();

    info!("Mounting the FUSE filesystem in the background...");

    // TODO: use tempdir for multiple instances
    let index_filename = args.database.join("files");
    let index_buffer = read_raw_buffer(index_filename).expect("Failed to read the index buffer!");
    let session = spawn_mount2(
        fs::BuildXYZ {
            index_buffer,
            ..Default::default()
        },
        "/tmp/buildxyz",
        &[],
    )
    .expect("Error spawning the FUSE filesystem in the background");

    info!("Running `{}`", args.cmd);

    let retry = Arc::new(AtomicBool::new(args.retry));
    // FIXME uninitialized values are bad.
    let current_child_pid = Arc::new(AtomicU32::new(0));
    if let [cmd, cmd_args @ ..] = &args.cmd.split_ascii_whitespace().collect::<Vec<&str>>()[..] {
        let run_join_handle = runner::spawn_instrumented_program(
            cmd.to_string(),
            // FIXME: ugh ugly
            cmd_args.to_vec().into_iter().map(|s| s.to_string()).collect(),
            std::env::vars().collect(),
            current_child_pid.clone(),
            retry.clone(),
        );

        // Main event loop
        // We wait for either stop signal or done signal
        loop {
            match recv_event.recv().expect("Failed to receive message") {
                EventMessage::Stop => {
                    stop_count += 1;
                    retry.store(false, Ordering::SeqCst);
                    let pid = Pid::from_raw(current_child_pid.load(Ordering::SeqCst) as i32);
                    ::nix::sys::signal::kill(pid, match stop_count {
                        2 => SIGTERM,
                        k if k >= 3 => SIGKILL,
                        _ => SIGINT
                    }).expect("Failed to interrupt the current underlying process");
                },
                EventMessage::Done => {
                    run_join_handle.join().expect("Failed to wait for the runner thread");
                    info!("Unmounting the filesystem...");
                    session.join();
                    break;
                }
            }
        }
    } else {
        todo!("Dependent type theory in Rust");
    }

    Ok(())
}
