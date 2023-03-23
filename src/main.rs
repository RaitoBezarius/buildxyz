use ::nix::sys::signal::Signal::{SIGINT, SIGKILL, SIGTERM};
use ::nix::unistd::Pid;
use cache::database::read_raw_buffer;
use clap::Parser;
use fuser::spawn_mount2;
use log::{debug, info};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;

// mod instrument;
mod cache;
mod fs;
mod interactive;
mod nix;
mod popcount;
mod runner;

pub enum EventMessage {
    Stop,
    Done,
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
    retry: bool,
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();

    stderrlog::new()
        //.module(module_path!())
        .verbosity(4)
        .init()
        .unwrap();

    // Signal to stop the current program
    // If sent twice, uses SIGKILL
    let (send_event, recv_event) = channel::<EventMessage>();
    let (send_fs_event, recv_fs_event) = channel();
    let (ui_join_handle, send_ui_event) = interactive::spawn_ui(send_fs_event.clone());
    let mut stop_count = 0;

    let ctrlc_event = send_event.clone();
    ctrlc::set_handler(move || {
        println!("stop count: {}", stop_count);
        info!("Ctrl-C received...");
        ctrlc_event
            .send(EventMessage::Stop)
            .expect("Failed to send Ctrl-C event to the main thread");
    })
    .expect("Failed to set Ctrl-C handler");
    // FIXME: register SIGTERM too.

    info!("Mounting the FUSE filesystem in the background...");

    let tmpdir = tempfile::tempdir().expect("Failed to create a temporary directory");

    // TODO: use tempdir for multiple instances
    // let index_filename = args.database.join("files");
    let session = spawn_mount2(
        fs::BuildXYZ {
            recv_fs_event,
            send_ui_event: send_ui_event.clone(),
            ..Default::default()
        },
        tmpdir
            .path()
            .to_str()
            .expect("Failed to convert the path to a string"),
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
            cmd_args
                .to_vec()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
            std::env::vars().collect(),
            current_child_pid.clone(),
            retry.clone(),
            send_event.clone(),
            tmpdir.path(),
        );

        // Main event loop
        // We wait for either stop signal or done signal
        loop {
            match recv_event.recv().expect("Failed to receive message") {
                EventMessage::Stop => {
                    stop_count += 1;
                    retry.store(false, Ordering::SeqCst);
                    send_ui_event
                        .send(interactive::UserRequest::Quit)
                        .expect("Failed to send message to UI thread");
                    let raw_pid = current_child_pid.load(Ordering::SeqCst) as i32;
                    let pid = Pid::from_raw(raw_pid);
                    if raw_pid != 0 {
                        debug!("ENOENT all pending fs requests...");
                        send_fs_event
                            .send(fs::FsEventMessage::IgnorePendingRequests)
                            .expect("Failed to send message to filesystem threads");
                        debug!("Will kill {:?}", pid);
                        ::nix::sys::signal::kill(
                            pid,
                            match stop_count {
                                2 => SIGTERM,
                                k if k >= 3 => SIGKILL,
                                _ => SIGINT,
                            },
                        )
                        .expect("Failed to interrupt the current underlying process");
                    } else {
                        send_event
                            .send(EventMessage::Done)
                            .expect("Failed to send event");
                    }
                }
                EventMessage::Done => {
                    // Ensure we quit the UI thread.
                    let _ = send_ui_event.send(interactive::UserRequest::Quit);
                    info!("Waiting for the runner & UI threads to exit...");
                    run_join_handle
                        .join()
                        .expect("Failed to wait for the runner thread");
                    ui_join_handle
                        .join()
                        .expect("Failed to wait for the UI thread");
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
