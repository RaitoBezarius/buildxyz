use ::nix::sys::signal::Signal::{SIGINT, SIGKILL, SIGTERM};
use ::nix::unistd::Pid;
use cache::database::read_raw_buffer;
use clap::Parser;
use fuser::spawn_mount2;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use std::io;
use std::iter;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use include_dir::{include_dir, Dir};

use crate::cache::StorePath;
use crate::nix::realize_path;
use crate::resolution::{
    load_resolution_db, merge_resolution_db, read_resolution_db, ResolutionDB, Resolution, Decision,
};

// mod instrument;
mod cache;
mod fs;
mod interactive;
mod nix;
mod popcount;
mod resolution;
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
    /// Say yes to everything except if it is recorded as ENOENT.
    #[arg(long = "automatic", default_value_t = false)]
    automatic: bool,
    /// No core resolution
    #[arg(long = "naked", default_value_t = false)]
    naked: bool,
    #[arg(long = "db", default_value_os = cache::cache_dir())]
    database: PathBuf,
    #[arg(long = "record-to")]
    resolution_record_filepath: Option<PathBuf>,
    #[arg(long = "resolutions-from")]
    custom_resolutions_filepath: Option<PathBuf>,
    /// In case of failures, retry automatically the invocation
    #[arg(long = "r", default_value_t = false)]
    retry: bool,
    /// Print ignored paths
    #[arg(long = "print-ignored-paths", default_value_t = false)]
    print_ignored_paths: bool
}

fn get_git_root() -> Option<std::path::PathBuf> {
    // TODO: `git` is not necessarily in the PATH, is it?
    let output = Command::new("git")
        .args(vec!["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(
            std::ffi::OsString::from_vec(output.stdout)
                .as_os_str()
                .into(),
        )
    } else {
        None
    }
}


static CORE_RESOLUTIONS: Dir = include_dir!("$BUILDXYZ_CORE_RESOLUTIONS");
lazy_static! {
    /// Here are the default search paths by order:
    ///   $XDG_DATA_DIR/buildxyz
    ///   "Git root"/.buildxyz if it exist.
    ///   Current working directory
    static ref DEFAULT_RESOLUTION_PATHS: Vec<PathBuf> = {
        let mut paths = Vec::new();
        let xdg_base_dir = xdg::BaseDirectories::with_prefix("buildxyz").unwrap();
        paths.push(
            xdg_base_dir.get_data_home()
        );
        if let Some(git_root) = get_git_root() {
            paths.push(
                git_root.join(".buildxyz")
            )
        }
        paths.push(
            std::env::current_dir().expect("Failed to get current working directory")
        );
        paths
    };
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
    let (ui_join_handle, send_ui_event) =
        interactive::spawn_ui(send_fs_event.clone(), args.automatic);
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

    let fuse_tmpdir = tempfile::tempdir().expect("Failed to create a temporary directory for the FUSE mountpoint");
    let fast_tmpdir = tempfile::tempdir().expect("Failed to create a temporary directory for the fast working tree");

    // Load all resolution databases in memory.
    // Reduce them by merging them in the provided priority order.
    // Load *core* resolutions first
    let core_resolution_db = if !args.naked { CORE_RESOLUTIONS.find("**/*.toml").unwrap()
        .into_iter()
        .map(|entry| CORE_RESOLUTIONS.get_file(entry.path()).expect("Failed to find a core resolution file inside the binary, corrupted binary?"))
        .filter_map(|file| read_resolution_db(file.contents_utf8().unwrap()))
        .fold(ResolutionDB::new(), |left, right| merge_resolution_db(left, right))
    } else { ResolutionDB::new() };

    let mut resolution_db = std::env::var("BUILDXYZ_RESOLUTION_PATH")
        .unwrap_or(String::new())
        .split(":")
        .into_iter()
        .map(PathBuf::from)
        // Default resolution paths are lowest priority.
        .chain(DEFAULT_RESOLUTION_PATHS.iter().cloned())
        .map(|searchpath| load_resolution_db(searchpath))
        .flatten() // Filter out all Nones.
        .fold(core_resolution_db, |left, right| {
            merge_resolution_db(left, right)
        });

    if let Some(custom_resolutions_filepath) = args.custom_resolutions_filepath {
        if let Some(custom_resolutions) = read_resolution_db(
            &std::fs::read_to_string(custom_resolutions_filepath).expect("Failed to read from custom resolution file")
        )
        {
            resolution_db = merge_resolution_db(resolution_db, custom_resolutions);
        }
    }

    if args.print_ignored_paths {
        println!("List of ignored paths:");
        for resolution in resolution_db.values() {
            let resolution::Resolution::ConstantResolution(data) = resolution;
            match data.decision {
                resolution::Decision::Ignore => {
                    println!("\t{}", data.requested_path);
                },
                _ => {}
            }
        }
        
        return Ok(());
    }


    let store_paths = resolution_db
        .values()
        .filter_map(|resolution| {
            debug!("store path: {:?}", resolution);
            match resolution {
                Resolution::ConstantResolution(data) => {
                    if let Decision::Provide(provide_data) = &data.decision {
                        return Some(provide_data.store_path.clone());
                    }
                }
            }

            None
        })
    .collect::<Vec<StorePath>>();

    for spath in store_paths {
        debug!("Ensuring that resolution {} is available in the Nix store", spath.as_str());
        if realize_path(spath.as_str().to_string()).is_err() {
            warn!("Failed to realize it, BuildXYZ may fail");
        }
    }

    let session = spawn_mount2(
        fs::BuildXYZ {
            recv_fs_event,
            send_ui_event: send_ui_event.clone(),
            resolution_record_filepath: args.resolution_record_filepath,
            resolution_db,
            fast_working_tree: fast_tmpdir.path().to_owned(),
            ..Default::default()
        },
        fuse_tmpdir
            .path()
            .to_str()
            .expect("Failed to convert the path to a string"),
        &[]

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
            fuse_tmpdir.path(),
            fast_tmpdir.path()
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
