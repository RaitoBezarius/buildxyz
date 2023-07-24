use log::{debug, error, info};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::{collections::HashMap, sync::mpsc::Sender};

use crate::EventMessage;

fn append_search_path(env: &mut HashMap<String, String>, key: &str, value: PathBuf, insert: bool) {
    let entry = env.entry(key.to_string()).and_modify(|env_path| {
        debug!("old env: {}={}", key, env_path);
        *env_path = format!(
            "{env_path}:{value}",
            env_path = env_path,
            value = value.display()
        );
    });

    if insert {
        entry.or_insert_with(|| {
            debug!("`{}` was not present before, injecting", key);
            format!("{}", value.display())
        });
    }
}

fn append_search_paths(env: &mut HashMap<String, String>,
    root_path: &Path) {
    let bin_path = root_path.join("bin");
    let pkgconfig_path = root_path.join("lib").join("pkgconfig");
    let library_path = root_path.join("lib");
    let include_path = root_path.join("include");
    let cmake_path = root_path.join("cmake");
    let aclocal_path = root_path.join("aclocal");
    let perl_path = root_path.join("perl");

    append_search_path(env, "PATH", bin_path, true);

    append_search_path(env, "PERL5LIB", perl_path, false);

    append_search_path(env, "PKG_CONFIG_PATH", pkgconfig_path, true);
    append_search_path(env, "CMAKE_INCLUDE_PATH", cmake_path, true);
    append_search_path(env, "ACLOCAL_PATH", aclocal_path, false);

    // Runtime libraries:
    // This is not a workable approach because DT_RUNPATH is after LD_LIBRARY_PATH
    // in priority. Anyway, on NixOS, most binaries comes with all the proper
    // libraries, on other OS, you must have them in your FHS.
    // Therefore, all that remains is handling foreign binaries.
    // This is taken care by composing buildxyz with nix-ld for example.
    // append_search_path(env, "LD_LIBRARY_PATH", library_path.clone(), false);

    // Build-time libraries
    append_search_path(env, "LIBRARY_PATH", library_path.clone(), true);

    env.entry("NIX_CFLAGS_COMPILE".to_string())
        .and_modify(|env_path| {
            debug!("old NIX_CFLAGS_COMPILE={}", env_path);
            *env_path = format!(
                "{env_path} -idirafter {include_path}",
                env_path = env_path,
                include_path = include_path.display()
            );
            debug!("new NIX_CFLAGS_COMPILE={}", env_path);
        });
}

pub fn spawn_instrumented_program(
    cmd: String,
    args: Vec<String>,
    mut env: HashMap<String, String>,
    current_child_pid: Arc<AtomicU32>,
    should_retry: Arc<AtomicBool>,
    send_to_main: Sender<EventMessage>,
    mountpoint: &Path,
    fast_working_root: &Path
) -> thread::JoinHandle<Option<i32>> {

    // Fast working tree
    append_search_paths(&mut env, fast_working_root);
    // FUSE
    append_search_paths(&mut env, mountpoint);

    thread::spawn(move || {
        loop {
            debug!("Spawning a child `{}`...", cmd);
            let mut child = Command::new(&cmd)
                .args(&args)
                .env_clear()
                .envs(&env)
                .spawn()
                .expect("Command failed to start");

            // Send our PID so we can get killed if needed.
            current_child_pid.store(child.id(), Ordering::SeqCst);
            debug!("Child spawned with PID {}, waiting...", child.id());
            let status = child.wait().expect("Failed to wait for child");
            let success = status.success();
            if !success && should_retry.load(Ordering::SeqCst) {
                info!("Command failed but it will be restarted soon.");
            } else if !success {
                error!("Command failed");
                send_to_main.send(EventMessage::Done)
                    .expect("Failed to send message to main thread");
                return status.code();
            } else {
                info!("Command ended successfully");
                send_to_main
                    .send(EventMessage::Done)
                    .expect("Failed to send message to main thread");
                return status.code();
            }
        }
    })
}
