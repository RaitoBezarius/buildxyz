use log::{debug, error, info};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::{collections::HashMap, sync::mpsc::Sender};

use crate::EventMessage;

fn append_search_path(env: &mut HashMap<String, String>, key: &str, value: PathBuf) {
    env.entry(key.to_string()).and_modify(|env_path| {
        debug!("old env: {}={}", key, env_path);
        *env_path = format!(
            "{env_path}:{value}",
            env_path = env_path,
            value = value.display()
        );
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
) -> thread::JoinHandle<()> {
    let bin_path = mountpoint.join("bin");
    let pkgconfig_path = mountpoint.join("lib").join("pkgconfig");
    let library_path = mountpoint.join("lib");
    let include_path = mountpoint.join("include");
    let cmake_path = mountpoint.join("cmake");
    let aclocal_path = mountpoint.join("aclocal");
    let perl_path = mountpoint.join("perl");

    append_search_path(&mut env, "PATH", bin_path);

    append_search_path(&mut env, "PERL5LIB", perl_path);

    append_search_path(&mut env, "PKG_CONFIG_PATH", pkgconfig_path);
    append_search_path(&mut env, "CMAKE_INCLUDE_PATH", cmake_path);
    append_search_path(&mut env, "ACLOCAL_PATH", aclocal_path);

    append_search_path(&mut env, "LD_LIBRARY_PATH", library_path.clone());

    env.entry("NIX_LDFLAGS".to_string()).and_modify(|env_path| {
        *env_path = format!(
            "{env_path} -L{library_path}",
            env_path = env_path,
            library_path = library_path.display()
        );
    });
    env.entry("NIX_CFLAGS_COMPILE".to_string())
        .and_modify(|env_path| {
            *env_path = format!(
                "{env_path} -isystem {include_path}",
                env_path = env_path,
                include_path = include_path.display()
            );
        });

    thread::spawn(move || {
        loop {
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
                break;
            } else {
                info!("Command ended successfully");
                send_to_main
                    .send(EventMessage::Done)
                    .expect("Failed to send message to main thread");
                break;
            }
        }
    })
}
