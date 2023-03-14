use std::ffi::OsStr;

pub mod database;
mod files;
mod frcode;
mod package;

pub use files::{FileNode, FileTreeEntry};
pub use package::StorePath;

pub fn cache_dir() -> &'static OsStr {
    let base = xdg::BaseDirectories::with_prefix("nix-index").unwrap();

    Box::leak(Box::new(base.get_cache_home())).as_os_str()
}
