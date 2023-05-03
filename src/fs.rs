use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use std::sync::mpsc::{channel, Receiver, Sender};

// TODO: is it Linux-specific?
use std::cell::RefCell;
use std::ffi::OsStr;

use fuser::{FileAttr, FileType, Filesystem};

use log::{debug, info, trace, warn};

use regex::bytes::Regex;

use crate::cache::database::{read_from_path, Reader};
use crate::cache::{cache_dir, FileNode, FileTreeEntry, StorePath};
use crate::interactive::UserRequest;
use crate::nix::{get_path_size, realize_path};
use crate::popcount::Popcount;

use crate::read_raw_buffer;
use crate::resolution::{db_to_human_toml, Decision, ProvideData, Resolution, ResolutionDB};

const UNIX_EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

pub enum FsEventMessage {
    /// Flush all current pending filesystem access to ENOENT
    IgnorePendingRequests,
    /// A package suggestion as a reply to a user interactive search
    PackageSuggestion(StorePath),
}

pub struct BuildXYZ {
    pub index_buffer: Vec<u8>,
    pub popcount_buffer: Popcount,
    /// resolution information for this instance
    pub resolution_db: ResolutionDB,
    /// where to write this instance resolutions
    pub resolution_record_filepath: Option<PathBuf>,
    /// recorded ENOENTs
    pub recorded_enoent: HashSet<(u64, String)>,
    pub global_dirs: HashMap<String, u64>,
    /// "global path" -> inode
    pub parent_prefixes: HashMap<u64, String>,
    /// inode -> "virtual paths"
    pub nix_paths: HashMap<u64, Vec<u8>>,
    /// inode -> nix store paths
    pub last_inode: RefCell<u64>,
    /// Receiver channel for commands
    pub recv_fs_event: Receiver<FsEventMessage>,
    /// Sender channel for UI requests
    pub send_ui_event: Sender<UserRequest>,
}

impl Default for BuildXYZ {
    fn default() -> Self {
        // Those are useless channels.
        let (_send, recv) = channel();
        let (send, _recv) = channel();

        BuildXYZ {
            popcount_buffer: serde_json::from_slice(include_bytes!("../popcount-graph.json"))
                .expect("Failed to deserialize the popcount graph"),
            index_buffer: read_raw_buffer(std::io::Cursor::new(include_bytes!(
                "../nix-index-files"
            )))
            .expect("Failed to deserialize the index buffer"),
            resolution_db: Default::default(),
            resolution_record_filepath: Default::default(),
            recorded_enoent: HashSet::new(),
            global_dirs: HashMap::new(),
            parent_prefixes: HashMap::new(),
            nix_paths: HashMap::new(),
            last_inode: 2.into(),
            recv_fs_event: recv,
            send_ui_event: send,
        }
    }
}

fn prompt_user(prompt: String) -> bool {
    loop {
        let mut answer = String::new();
        println!("{}", prompt);
        io::stdin()
            .read_line(&mut answer)
            .ok()
            .expect("Failed to read line");

        print!("{}", answer.as_str());

        match answer.as_str() {
            "y" => return true,
            "n" => return false,
            _ => {}
        }
    }
}

#[inline]
fn build_fake_fattr(ino: u64, kind: FileType) -> FileAttr {
    fuser::FileAttr {
        kind,
        ino,
        size: 1,
        blocks: 1,
        blksize: 1,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        flags: 0,
        uid: 0,
        gid: 0,
        nlink: 1,
        rdev: 0,
        perm: 777,
    }
}

fn is_file_or_symlink<T>(n: &FileNode<T>) -> bool {
    match n {
        FileNode::Regular { .. } => true,
        FileNode::Symlink { .. } => true,
        FileNode::Directory { .. } => false,
    }
}

fn is_dir<T>(n: &FileNode<T>) -> bool {
    // FIXME
    // investigate interaction with symlinkJoin
    match n {
        FileNode::Regular { .. } => false,
        FileNode::Symlink { .. } => true, // /nix/store/b6ks67mjvh2hzy3k1rnvmlri6p63b4vj-python3-3.9.12-env/include/boost is a symlink for example.
        FileNode::Directory { .. } => true,
    }
}

// TODO: two policies — return fake information or lstat the file and return true information
impl<T> Into<fuser::FileAttr> for FileNode<T> {
    fn into(self) -> fuser::FileAttr {
        let kind = match self {
            Self::Regular { .. } => fuser::FileType::Symlink, // No matter what, we want readlink,
            // not read.
            Self::Symlink { .. } => fuser::FileType::Symlink,
            Self::Directory { .. } => fuser::FileType::Directory,
        };

        build_fake_fattr(1, kind)
    }
}

/// This will go through all candidates
/// according to the sort function order
/// and return the best
/// It will perform some debug asserts on the list.
fn extract_optimal_path<F>(
    candidates: &mut Vec<(StorePath, FileTreeEntry)>,
    sort_key_function: F,
) -> (&StorePath, &FileTreeEntry)
where
    F: FnMut(&(StorePath, FileTreeEntry)) -> i32,
{
    // 1. There cannot be a folder and a file at the same time in `candidates`
    debug_assert!(
        candidates
            .into_iter()
            .all(|(_, c)| is_file_or_symlink(&c.node))
            || candidates.into_iter().all(|(_, c)| is_dir(&c.node)),
        "either candidates are all directories, either all files, not in-between."
    );

    // FIXME: is it enough for the ranking algorithm?
    candidates.sort_by_cached_key(sort_key_function);

    let (store_path, ft_entry) = candidates.first().unwrap();

    (store_path, ft_entry)
    /*let mut fattr: fuser::FileAttr = ft_entry.node.clone().into();
    fattr.ino = offered_inode;

    // This dance is necessary because ft_entry.path starts with a /
    // and join will keep only the second arg for an absolute path.
    let nix_store_dir = store_path.as_str().into_owned();
    let nix_store_dir_path = Path::new(&nix_store_dir);
    let nix_path = nix_store_dir_path.join(
        String::from_utf8_lossy(&ft_entry.path)
            .into_owned()
            .strip_prefix("/")
            .unwrap(),
    );
    (store_path, fattr, nix_path.as_os_str().as_bytes().to_vec())*/
}

impl BuildXYZ {
    fn allocate_inode(&self) -> u64 {
        *self.last_inode.borrow_mut() += 1;
        *self.last_inode.borrow() - 1
    }

    fn build_in_construction_path(&self, parent: u64, name: &OsStr) -> PathBuf {
        let prefix = Path::new(
            self.parent_prefixes
                .get(&parent)
                .expect("Unknown parent inode!"),
        );

        prefix.join(name)
    }

    fn record_resolution(&mut self, parent: u64, name: &OsStr, decision: Decision) {
        let current_path = self
            .build_in_construction_path(parent, name)
            .to_string_lossy()
            .to_string();
        trace!("Recording {} for {:?}", current_path, decision);
        self.resolution_db.insert(
            current_path.clone(),
            Resolution::ConstantResolution(crate::resolution::ResolutionData {
                requested_path: current_path,
                decision,
            }),
        );
    }

    fn get_resolution(&self, parent: u64, name: &OsStr) -> Option<&Resolution> {
        let current_path = self
            .build_in_construction_path(parent, name)
            .to_string_lossy()
            .to_string();
        self.resolution_db.get(&current_path)
    }

    fn get_decision(&self, parent: u64, name: &OsStr) -> Option<&Decision> {
        match self.get_resolution(parent, name) {
            Some(Resolution::ConstantResolution(data)) => Some(&data.decision),
            _ => None,
        }
    }

    /// Serve the path as an answer to the filesystem
    /// It realizes the Nix path if it's not already.
    fn serve_path(
        &mut self,
        nix_path: Vec<u8>,
        requested_path: PathBuf,
        attribute: fuser::FileAttr,
        reply: fuser::ReplyEntry,
    ) {
        let nix_path_as_str = String::from_utf8_lossy(&nix_path);
        trace!("{}: {:?}", nix_path_as_str, attribute);
        self.parent_prefixes
            .insert(attribute.ino, requested_path.to_string_lossy().to_string());

        realize_path(nix_path_as_str.into())
            .expect("Nix path should be realized, database seems incoherent with Nix store.");

        self.nix_paths.insert(attribute.ino, nix_path);

        reply.entry(&Duration::from_secs(60 * 20), &attribute, attribute.ino);
    }

    /// Runs a query using our index
    fn search_in_index(&self, requested_path: &PathBuf) -> Vec<(StorePath, FileTreeEntry)> {
        let escaped_path = regex::escape(&requested_path.to_string_lossy());
        debug!(
            "looking for: `{}$` in Nix database",
            requested_path.to_string_lossy(),
        );
        let now = Instant::now();
        // TODO: put me behind Arc
        let db = Reader::from_buffer(self.index_buffer.clone()).expect("Failed to open database");

        let candidates: Vec<(StorePath, FileTreeEntry)> = db
            .query(&Regex::new(format!(r"^/{}$", escaped_path).as_str()).unwrap())
            .run()
            .expect("Failed to query the database")
            .into_iter()
            .map(|result| result.expect("Failed to obtain candidate"))
            .filter(|(spath, _)| spath.origin().toplevel) // It must be a top-level path, otherwise
            // it is propagated, so not to consider.
            .collect();
        trace!("{:?}", candidates);
        debug!("search took {:.2?}", now.elapsed());

        candidates
    }

    /// Register known "FHS" structure
    /// Assume parents are already created.
    fn mkdir_fhs_directory(&mut self, path: &str) {
        let inode = self.allocate_inode();
        self.parent_prefixes.insert(inode, path.to_string());
        self.global_dirs.insert(path.to_string(), inode);
    }
}

impl Filesystem for BuildXYZ {
    fn init(
        &mut self,
        _req: &fuser::Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), i32> {
        self.parent_prefixes.insert(1, "".to_string());
        // Create bin, lib, include, pkg-config inodes
        // TODO: Keep this list synchronized with created search paths in runner.rs?
        [
            "bin",
            "include",
            "perl",
            "aclocal",
            "cmake",
            "lib",
            "lib/pkgconfig",
        ]
        .into_iter()
        .for_each(|c| self.mkdir_fhs_directory(c));
        info!(
            "Loaded {} resolutions from the database.",
            self.resolution_db.len()
        );
        Ok(())
    }

    fn destroy(&mut self) {
        if let Some(filepath) = &self.resolution_record_filepath {
            debug!(
                "Writing {} resolutions on disk...",
                self.resolution_db.len()
            );
            // Write this resolution on disk.
            std::fs::write(
                filepath,
                toml::to_string_pretty(&db_to_human_toml(&self.resolution_db))
                    .expect("Failed to serialize in a human-way the resolution database"),
            )
            .expect("Failed to write resolution data");
        }
    }

    fn lookup(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let target_path = self.build_in_construction_path(parent, name);

        // global directory
        if let Some(inode) = self
            .global_dirs
            .get(&target_path.to_string_lossy().to_string())
        {
            trace!(
                "global directory hit: {}",
                &target_path.to_string_lossy().to_string()
            );
            reply.entry(
                &Duration::from_secs(60 * 60),
                &build_fake_fattr(*inode, FileType::Directory),
                *inode,
            );
            return;
        }

        // No other global directories.
        if parent == 1 {
            return reply.error(nix::errno::Errno::ENOENT as i32);
        }

        // Fast path: ignore temporarily recorded ENOENTs.
        if self
            .recorded_enoent
            .contains(&(parent, name.to_string_lossy().to_string()))
        {
            return reply.error(nix::errno::Errno::ENOENT as i32);
        }

        // Fast path: general resolutions
        let path_provide_data: Option<&ProvideData> = match self.get_decision(parent, name) {
            Some(Decision::Provide(data)) => Some(data),
            Some(Decision::Ignore) => return reply.error(nix::errno::Errno::ENOENT as i32),
            _ => None,
        };

        if let Some(data) = path_provide_data {
            trace!("FAST PATH - Decision already exist in current database");
            let nix_path = data
                .store_path
                .join(data.file_entry_name.clone().into())
                .into_owned()
                .as_str()
                .as_bytes()
                .to_vec();
            let ft_attribute = build_fake_fattr(self.allocate_inode(), data.kind);
            return self.serve_path(nix_path, target_path, ft_attribute, reply);
        }

        let mut candidates = self.search_in_index(&target_path);

        if !candidates.is_empty() {
            let (store_path, ft_entry) =
                extract_optimal_path(&mut candidates, |(store_path, _)| {
                    trace!(
                        "extracting pop for {}: {}",
                        store_path.as_str(),
                        store_path.origin().attr
                    );
                    // Highest popularity comes first, so inverted popularity works here.
                    let pop = -(*self
                        .popcount_buffer
                        .native_build_inputs
                        .get(&store_path.as_str().to_string())
                        .unwrap_or(&0) as i32);
                    trace!("pop: {pop}");
                    pop
                });

            // Ask the user if he want to provide this dependency?
            let mut ft_attribute: fuser::FileAttr = ft_entry.node.clone().into();
            let file_entry_name = String::from_utf8_lossy(&ft_entry.path).to_string();
            let nix_path = store_path
                .join_entry(ft_entry.clone())
                .into_owned()
                .as_str()
                .as_bytes()
                .to_vec();
            let spath = store_path.clone();
            self.send_ui_event
                .send(UserRequest::InteractiveSearch(candidates.clone(), spath))
                .expect("Failed to send UI thread a message");

            // FIXME: timeouts?
            match self.recv_fs_event.recv() {
                Ok(FsEventMessage::PackageSuggestion(pkg)) => {
                    debug!("prompt reply: {:?}", pkg);
                    // Allocate a file attribute for this file entry.
                    ft_attribute.ino = self.allocate_inode();
                    // TODO: use actually pkg, for now, it's guaranteed pkg == suggested candidate.
                    self.record_resolution(
                        parent,
                        name,
                        Decision::Provide(ProvideData {
                            file_entry_name,
                            kind: ft_attribute.kind,
                            store_path: pkg,
                        }),
                    );
                    return self.serve_path(nix_path, target_path, ft_attribute, reply);
                }
                Ok(FsEventMessage::IgnorePendingRequests) | _ => {
                    debug!("ENOENT received from user");
                    self.record_resolution(parent, name, Decision::Ignore);
                    self.recorded_enoent
                        .insert((parent, name.to_string_lossy().to_string()));
                    return reply.error(nix::errno::Errno::ENOENT as i32);
                }
            };
        } else {
            // This file potentially don't exist at all
            // But it is also possible we just do not have the package for it yet.
            // FIXME: provide proper heuristics for this.
            debug!("not found in database, recording this ENOENT.");
            self.recorded_enoent
                .insert((parent, name.to_string_lossy().to_string()));
            return reply.error(nix::errno::Errno::ENOENT as i32);
        }
    }

    fn readlink(&mut self, _req: &fuser::Request<'_>, ino: u64, reply: fuser::ReplyData) {
        if let Some(nix_path) = self.nix_paths.get(&ino) {
            // Ensure the path is realized, it could have been gc'd between the lookup and the
            // readlink.
            if realize_path(String::from_utf8_lossy(&nix_path).into()).is_err() {
                warn!(
                    "Failed to realize {} during readlink, it was supposed to be realizable!",
                    String::from_utf8_lossy(&nix_path)
                );
                reply.error(nix::errno::Errno::ENOENT as i32);
            } else {
                reply.data(nix_path);
            }
        } else {
            warn!("Attempt to read a non-existent Nix path, ino={}", ino);
            reply.error(nix::errno::Errno::ENOENT as i32);
        }
    }
}
