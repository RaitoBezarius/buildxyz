use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

// TODO: is it Linux-specific?
use std::ffi::OsStr;
use std::os::unix::prelude::OsStrExt;

use fuser::{FileAttr, FileType, Filesystem};

use log::{debug, info, trace};

use regex::bytes::Regex;

use crate::cache::database::{read_raw_buffer, Reader};
use crate::cache::{cache_dir, FileNode, FileTreeEntry, StorePath};
use crate::nix::{get_path_size, realize_path};
use crate::popcount::Popcount;

const UNIX_EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

pub struct BuildXYZ {
    pub index_buffer: Vec<u8>,
    pub popcount_buffer: Popcount,
    pub global_dirs: HashMap<String, u64>,
    /// "global path" -> inode
    pub parent_prefixes: HashMap<u64, String>,
    /// inode -> "virtual paths"
    pub nix_paths: HashMap<u64, Vec<u8>>,
    /// inode -> nix store paths
    pub last_inode: u64,
}

impl Default for BuildXYZ {
    fn default() -> Self {
        BuildXYZ {
            popcount_buffer: serde_json::from_reader(BufReader::new(
                File::open("popcount-graph.json").expect("Failed to open popcount-graph.json"),
            ))
            .expect("Failed to read popcount graph in JSON"),
            index_buffer: read_raw_buffer(Path::new(cache_dir()).join("files"))
                .expect("Failed to read the index buffer"),
            global_dirs: HashMap::new(),
            parent_prefixes: HashMap::new(),
            nix_paths: HashMap::new(),
            last_inode: 2,
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

fn extract_optimal_file_attr<F>(
    candidates: &mut Vec<(StorePath, FileTreeEntry)>,
    offered_inode: u64,
    sort_key_function: F,
) -> (fuser::FileAttr, Vec<u8>)
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

    let mut fattr: fuser::FileAttr = ft_entry.node.clone().into();
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
    (fattr, nix_path.as_os_str().as_bytes().to_vec())
}

impl BuildXYZ {
    fn allocate_inode(&mut self) -> u64 {
        self.last_inode += 1;

        self.last_inode - 1
    }
}

impl Filesystem for BuildXYZ {
    fn init(
        &mut self,
        _req: &fuser::Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), i32> {
        self.parent_prefixes.insert(0, "".to_string());
        // Create bin, lib, include, pkg-config inodes
        for fhs_dir in ["bin", "lib", "include", "pkgconfig"] {
            let inode = self.allocate_inode();
            self.parent_prefixes.insert(inode, fhs_dir.to_string());
            self.global_dirs.insert(fhs_dir.to_string(), inode);
        }
        Ok(())
    }

    fn destroy(&mut self) {}

    fn lookup(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        // global directory
        if let Some(inode) = self.global_dirs.get(&name.to_string_lossy().to_string()) {
            if parent == 1 {
                trace!("global directory hit: {}", name.to_string_lossy());
                reply.entry(
                    &Duration::from_secs(60 * 60),
                    &build_fake_fattr(*inode, FileType::Directory),
                    *inode,
                );
                return;
            }
        }

        // No other global directories.
        if parent == 1 {
            reply.error(nix::errno::Errno::ENOENT as i32);
            return;
        }

        // TODO: put me behind Arc
        let db = Reader::from_buffer(self.index_buffer.clone()).expect("Failed to open database");
        let prefix = Path::new(
            self.parent_prefixes
                .get(&parent)
                .expect("Unknown parent inode!"),
        );
        let target_path = prefix.join(name);
        debug!(
            "looking for: {}$ in Nix database (parent inode: {parent})",
            target_path.to_string_lossy()
        );
        let now = Instant::now();
        let mut candidates: Vec<(StorePath, FileTreeEntry)> = db
            .query(&Regex::new(format!(r"{}$", target_path.to_string_lossy()).as_str()).unwrap())
            .run()
            .expect("Failed to query the database")
            .into_iter()
            .map(|result| result.expect("Failed to obtain candidate"))
            .collect();
        trace!("{:?}", candidates);
        debug!("search took {:.2?}", now.elapsed());

        if !candidates.is_empty() {
            // TODO: immutable borrow stuff with allocate_inode()
            self.last_inode += 1;
            // FileAttr based on available candidates
            // candidates.sort_by_cached_key(|(store_path, _)| {
            //     let stpath_str = store_path.as_str();
            //     get_path_size(&stpath_str, crate::nix::StoreKind::Local)
            //     .or_else(|| get_path_size(&stpath_str, crate::nix::StoreKind::Remote("https://cache.nixos.org".to_string())))
            //     .or(Some(usize::MAX))
            // });

            let (attr, nix_path) = extract_optimal_file_attr(
                &mut candidates,
                self.last_inode - 1,
                |(store_path, _)| {
                    trace!("extracting pop for {}", store_path.as_str());
                    // Highest popularity comes first, so inverted popularity works here.
                    let pop = -(*self
                        .popcount_buffer
                        .native_build_inputs
                        .get(&store_path.as_str().to_string())
                        .unwrap_or(&0) as i32);
                    trace!("pop: {pop}");
                    pop
                },
            );
            trace!("{}: {:?}", String::from_utf8_lossy(&nix_path), attr);
            self.parent_prefixes.insert(
                self.last_inode - 1,
                target_path.to_string_lossy().to_string(),
            );
            // Realize the path
            // TODO: can I realize it after answering to the caller?
            realize_path(String::from_utf8_lossy(&nix_path).into())
                .expect("Nix path should be realized, database seems incoherent with store");
            self.nix_paths.insert(self.last_inode - 1, nix_path);
            // 20 mns ttl
            reply.entry(&Duration::from_secs(60 * 20), &attr, attr.ino);
        } else {
            // This file potentially don't exist at all
            // But it is also possible we just do not have the package for it yet.
            // FIXME: provide proper heuristics for this.
            debug!("not found");
            reply.error(nix::errno::Errno::ENOENT as i32);
        }
    }

    fn readlink(&mut self, _req: &fuser::Request<'_>, ino: u64, reply: fuser::ReplyData) {
        if let Some(nix_path) = self.nix_paths.get(&ino) {
            // Ensure the path is realized, it could have been gc'd between the lookup and the
            // readlink.
            if realize_path(String::from_utf8_lossy(&nix_path).into()).is_err() {
                reply.error(nix::errno::Errno::ENOENT as i32);
            } else {
                reply.data(nix_path);
            }
        } else {
            reply.error(nix::errno::Errno::ENOENT as i32);
        }
    }
}
