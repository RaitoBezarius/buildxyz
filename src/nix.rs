use log::trace;
use serde::Deserialize;
use std::process::{Command, Stdio};

use error_chain::{bail, error_chain};

pub enum StoreKind {
    Local,
    Remote(String),
}

error_chain! {
    errors { InvalidPath }
}

/// Ask the store to realize the provided path.
pub fn realize_path(path: String) -> Result<()> {
    // TODO: send back this information to the meta-panel of the TUI
    let output = Command::new("nix-store")
        .arg("--realize")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .expect("Failed to realize store based on nix-store --realize");

    if output.status.success() {
        Ok(())
    } else {
        // TODO: more precise errors.
        bail!(ErrorKind::InvalidPath)
    }
}

#[derive(Deserialize)]
struct PathInfo {
    #[serde(rename = "closureSize")]
    closure_size: Option<usize>,
}

/// Returns `nix path-info -S <path> --store <store> if there's any remote store.
/// If the path is invalid, None is returned.
/// This returns the closure size.
pub fn get_path_size(path: &str, store: StoreKind) -> Option<usize> {
    let mut cmd0 = Command::new("nix");
    let mut cmd = cmd0.arg("path-info").arg("--json").arg("-S").arg(path);

    cmd = match store {
        StoreKind::Local => cmd,
        StoreKind::Remote(remote_store) => cmd.arg("--store").arg(remote_store),
    };

    let output = cmd.output().expect("Failed to extract path information");

    trace!(
        "nix path-info output: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    if output.status.success() {
        let pinfos: Vec<PathInfo> =
            serde_json::from_slice(&output.stdout).expect("Valid JSON from nix path-info --json");
        pinfos.first().expect("At least one path-info").closure_size
    } else {
        None
    }
}
