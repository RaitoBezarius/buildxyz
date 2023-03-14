use log::{debug, error, info};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::{collections::HashMap, sync::mpsc::Sender};

pub fn spawn_instrumented_program(
    cmd: String,
    args: Vec<String>,
    mut env: HashMap<String, String>,
    current_child_pid: Arc<AtomicU32>,
    should_retry: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    env.entry("PATH".to_string()).and_modify(|env_path| {
        *env_path = format!("{env_path}:/tmp/buildxyz/bin");
    });
    env.entry("PKG_CONFIG_PATH".to_string())
        .and_modify(|env_path| {
            *env_path = format!("{env_path}:/tmp/buildxyz/pkgconfig");
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
                break;
            }
        }
    })
}
