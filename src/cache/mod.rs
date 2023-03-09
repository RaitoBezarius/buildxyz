use std::ffi::OsStr;

mod frcode;
mod files;
mod package;
pub mod database;

pub use package::StorePath;
pub use files::{FileTreeEntry, FileNode};

pub fn cache_dir() -> &'static OsStr {
    let base = xdg::BaseDirectories::with_prefix("nix-index").unwrap();

    Box::leak(Box::new(base.get_cache_home())).as_os_str()
}

