use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use std::sync::mpsc::{channel, Receiver, Sender};

// TODO: is it Linux-specific?
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};

use std::os::unix::ffi::OsStringExt;

use fuser::{FileAttr, FileType, Filesystem};

use log::{debug, info, trace, warn};

use regex::bytes::Regex;
use walkdir::WalkDir;

use crate::cache::database::Reader;
use crate::cache::{FileNode, FileTreeEntry, StorePath};
use crate::interactive::UserRequest;
use crate::nix::realize_path;
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
    /// inode -> "virtual foreign paths" (on another filesystem)
    pub redirections: HashMap<u64, Vec<u8>>,
    /// fast working tree for subgraph extraction
    pub fast_working_tree: PathBuf,
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
            fast_working_tree: String::new().into(),
            nix_paths: HashMap::new(),
            redirections: HashMap::new(),
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

/// This will create all the directories and symlink only the leaves.
/// It will fail in case of incompatibility.
fn shadow_symlink_leaves(src_dir: &Path, target_dir: &Path, excluded_dirs: &Vec<&str>) -> std::io::Result<()> {
    // Do not follow symlinks
    // Otherwise, you will get an entry.path() which does not share a base prefix with src_dir
    // Therefore, you don't know where to send it.
    // Symlink compression should be done only at the end as an optimization if needed.
    // TODO: detect circular references.
    trace!("shadow symlinking {} -> {}...", src_dir.display(), target_dir.display());
    for entry in WalkDir::new(src_dir).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        // ensure target_dir.join(entry modulo src_dir) is a directory
        // or a symlink.
        let ft = entry.file_type();
        let suffix_path = entry.path().strip_prefix(src_dir).unwrap();
        let target_path = target_dir.join(suffix_path);

        // If the target path already exist, ignore this.
        if target_path.exists() {
            continue;
        }

        // Skip stuff like nix-support/*
        if excluded_dirs.iter().any(|forbidden_dir| suffix_path.starts_with(forbidden_dir)) {
            trace!("skipped {}", suffix_path.display());
            continue;
        }
        if ft.is_dir() {
            trace!("mkdir -p {} based on {}", target_path.display(), entry.path().display());
            std::fs::create_dir_all(target_path)?;
        } else if ft.is_file() {
            trace!("symlink {} -> {}", entry.path().display(), target_path.display());
            std::os::unix::fs::symlink(entry.path(), target_path)?;
        } else if ft.is_symlink() {
            // Two things has to be done
            // 1. Resolve completely the entry into resolved_target
            // 2. Recurse on resolved_target -> target_path
            // 2. Symlink target_path -> resolved_target
            let mut resolved_target = std::fs::read_link(entry.path())?;
            while resolved_target.is_symlink() {
                resolved_target = std::fs::read_link(entry.path())?;
            }
            // Now, `resolved_target` is completely resolved.
            // Either, it's relative, either it's absolute.
            // If it's relative, we correct it to an absolute link, by concatenating
            // $src_dir/$resolved_target.
            // If it's absolute, we proceed to recurse into it.
            if resolved_target.is_relative() {
                resolved_target = entry.path().parent().expect("Expected a symlink parented by at least /").join(resolved_target);
            }
            trace!("encountered an internal symlink: {} -> {}, symlinking or recursing depending on file type", entry.path().display(), resolved_target.display());
            // If it's a dir, recurse the symlinkage
            if resolved_target.is_dir() {
                trace!("recursing into the symlink {} -> {} for directory symlinkage", entry.path().display(), resolved_target.display());
                shadow_symlink_leaves(
                    &resolved_target,
                    &target_path,
                    excluded_dirs
                )?;
            }
            else if resolved_target.is_file() {
                trace!("symlink ({} ->) {} -> {}", entry.path().display(), resolved_target.display(), target_path.display());
                std::os::unix::fs::symlink(entry.path(), target_path)?;
            }
        }
    }

    Ok(())
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
    
    // Shadow symlink in the fast working tree
    // this Nix path
    fn extend_fast_working_tree(
        &mut self,
        store_path: &StorePath
    ) {
        let npath: PathBuf = OsString::from_vec(store_path.as_str().as_bytes().to_vec()).into();
        debug!("Shadow symlinking all the leaves {} -> {}", npath.display(), self.fast_working_tree.display());
        // We do not want to symlink nix-support
        shadow_symlink_leaves(&npath, &self.fast_working_tree, &vec![
            "nix-support"
        ])
            .expect("Failed to shadow symlink the Nix path inside the fast working tree, potential incompatibility");
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

    /// Redirect to a filesystem file
    /// via symlink
    fn redirect_to_fs(
        &mut self,
        reply: fuser::ReplyEntry,
        onfs_path: PathBuf
    ) {
        trace!("redirecting to {} on another filesystem", onfs_path.display());

        let ft_attribute = build_fake_fattr(self.allocate_inode(),
            fuser::FileType::Symlink);
        self.redirections.insert(ft_attribute.ino, onfs_path.to_string_lossy().as_bytes().to_vec());
        reply.entry(&Duration::from_secs(60 * 20), &ft_attribute, ft_attribute.ino);
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

// Allow parallel calls to lookup() as it should be fine.
const FUSE_CAP_PARALLEL_DIROPS: u32 = 1 << 18;
// Cache the symlinks we provide in the page cache.
const FUSE_CAP_CACHE_SYMLINKS: u32 = 1 << 23;

impl Filesystem for BuildXYZ {
    fn init(
        &mut self,
        _req: &fuser::Request<'_>,
        config: &mut fuser::KernelConfig,
    ) -> Result<(), i32> {
        // https://www.kernel.org/doc/html/latest/filesystems/fuse.html
        // https://libfuse.github.io/doxygen/fuse__common_8h.html
        config
            .add_capabilities(FUSE_CAP_PARALLEL_DIROPS)
            .map_err(|err| -(err as i32))?;
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

        let store_paths = self.resolution_db
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

        info!(
            "Will fast extend {} store paths.",
            store_paths.len()
        );

        for spath in store_paths {
            debug!("{} being extended in the working tree", spath.as_str());
            self.extend_fast_working_tree(&spath);
        }

        info!(
            "Fast working tree ready based on the resolutions."
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

        // Fast path: fast working tree
        // Rebase the target path based on the working tree structure
        if self.fast_working_tree.join(&target_path).exists() {
            trace!("FAST PATH — Path already exist in the fast working tree");
            return self.redirect_to_fs(reply, self.fast_working_tree.join(target_path));
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
            // FIXME: that's very ugly.
            let spath2 = store_path.clone();
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
                    // Now, we want to extract the whole subgraph
                    // Instead of trying to figure out that subgraph
                    // We can grab the Nix path and extend the fast working tree with it
                    // à la lndir.
                    self.extend_fast_working_tree(&spath2);
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
        }
        else if let Some(redirection_path) = self.redirections.get(&ino) {
            reply.data(redirection_path);
        } else {
            warn!("Attempt to read a non-existent Nix path, ino={}", ino);
            reply.error(nix::errno::Errno::ENOENT as i32);
        }
    }
}
