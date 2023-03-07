use fuser::{spawn_mount2, Filesystem};
use log::{debug, trace, info};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::process::{Command, Stdio};
use std::time::{SystemTime, Duration};
use clap::Parser;
use std::io;
use memfile::MemFile;
use regex::bytes::Regex;

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

fn cache_dir() -> &'static OsStr {
    let base = xdg::BaseDirectories::with_prefix("nix-index").unwrap();

    Box::leak(Box::new(base.get_cache_home())).as_os_str()
}


// 2 directories:
// - FUSE filesystem for negative lookups
// - normal filesystem for building the build environment (buildEnv)

struct BuildXYZ {
    index_filename: PathBuf,
    parent_prefixes: HashMap<u64, String>,
    last_inode: u64
}

impl Default for BuildXYZ {
    fn default() -> Self {
        BuildXYZ {
            index_filename: cache_dir().into(),
            parent_prefixes: HashMap::new(),
            last_inode: 0
        }
    }
}


fn is_file_or_symlink<T>(n: &cache::FileNode<T>) -> bool {
    match n {
        cache::FileNode::Regular { .. } => true,
        cache::FileNode::Symlink { .. } => true,
        cache::FileNode::Directory { .. } => false
    }
}

fn is_dir<T>(n: &cache::FileNode<T>) -> bool {
    // FIXME
    // investigate interaction with symlinkJoin
    match n {
        cache::FileNode::Regular { .. } => false,
        cache::FileNode::Symlink { .. } => true, // /nix/store/b6ks67mjvh2hzy3k1rnvmlri6p63b4vj-python3-3.9.12-env/include/boost is a symlink for example.
        cache::FileNode::Directory { .. } => true
    }
}

// TODO: two policies — return fake information or lstat the file and return true information
impl<T> Into<fuser::FileAttr> for cache::FileNode<T> {
    fn into(self) -> fuser::FileAttr {
        let kind = match self {
            Self::Regular { .. } => fuser::FileType::RegularFile,
            Self::Symlink { .. } => fuser::FileType::Symlink,
            Self::Directory { .. } => fuser::FileType::Directory
        };
        let only_time_that_exist = SystemTime::UNIX_EPOCH;

        fuser::FileAttr {
            kind,
            ino: 1,
            size: 1,
            blocks: 1,
            blksize: 1,
            atime: only_time_that_exist,
            mtime: only_time_that_exist,
            crtime: only_time_that_exist,
            ctime: only_time_that_exist,
            flags: 1,
            uid: 1,
            gid: 1, 
            nlink: 1,
            rdev: 1,
            perm: 1
        }
    }
}

fn extract_optimal_file_attr(candidates: &Vec<cache::FileTreeEntry>, offered_inode: u64) -> fuser::FileAttr {
    // 1. There cannot be a folder and a file at the same time in `candidates`
    debug_assert!(
        candidates.into_iter().all(|c| is_file_or_symlink(&c.node)) ||
        candidates.into_iter().all(|c| is_dir(&c.node)),
        "either candidates are all directories, either all files, not in-between."
    );

    // Ranking algorithm
    // For now, the first?

    let mut fattr: fuser::FileAttr = candidates.first().unwrap().node.clone().into();
    fattr.ino = offered_inode;

    fattr
}

impl BuildXYZ {
    fn allocate_inode(&mut self) -> u64 {
        self.last_inode += 1;

        self.last_inode - 1
    }
}

impl Filesystem for BuildXYZ {
    fn init(&mut self, _req: &fuser::Request<'_>, _config: &mut fuser::KernelConfig) -> Result<(), i32> {
        // Create lib, include inodes
        for fhs_dir in ["lib", "include"] {
            let inode = self.allocate_inode();
            self.parent_prefixes.insert(inode, fhs_dir.to_string());
        }
        Ok(())
    }

    fn destroy(&mut self) {
    }

    fn lookup(&mut self, _req: &fuser::Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEntry) {
        let db = cache::database::Reader::open(&self.index_filename).expect("Failed to open database");
        let prefix = self.parent_prefixes.get(&parent).expect("Unknown parent inode!");
        let lossy_name = name.to_string_lossy();
        debug!("looking for: {prefix}/{lossy_name}$ in Nix database (parent inode: {parent})");
        let candidates: Vec<(cache::StorePath, cache::FileTreeEntry)> = db.query(&Regex::new(format!(r"{prefix}/{lossy_name}$").as_str()).unwrap())
            .run()
            .expect("Failed to query the database")
            .into_iter()
            .map(|result| result.expect("Failed to obtain candidate"))
            .collect();
        trace!("{:?}", candidates);

        if !candidates.is_empty() {
            // FileAttr based on available candidates
            let attr = extract_optimal_file_attr(&candidates
                .into_iter()
                .map(|(_a, b)| b)
                .collect(),
                self.allocate_inode());
            // 20 mns ttl
            trace!("{:?}", attr);
            reply.entry(&Duration::from_secs(60*20), &attr, attr.ino);
        } else {
            // This file potentially don't exist at all
            // But it is also possible we just do not have the package for it yet.
            // FIXME: provide proper heuristics for this.
            debug!("not found");
            reply.error(nix::errno::Errno::ENOENT as i32);
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    cmd: String,
    #[arg(long = "db", default_value_os = cache_dir())]
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
    let session = spawn_mount2(
        BuildXYZ { index_filename, ..Default::default() },
        "/tmp/buildxyz",
        &[]
    ).expect("Error spawning the FUSE filesystem in the background");

    info!("Running `{}`", args.cmd);

    // 1. Setup the environment variables
    // PATH_XXX="build env:negative lookup folder (FUSE)"

    // Let's keep PATH for now.
    let instrumented_env: HashMap<String, String> = std::env::vars()
        .filter(|&(ref k, _)|
            k == "PATH"
        ).collect();

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
