use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

use crate::cache::StorePath;
/// Resolution is data that enable the tool to automate a situation where
/// a manual decision has to be taken.

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ProvideData {
    pub kind: fuser::FileType,
    pub file_entry_name: String,
    pub store_path: StorePath,
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq, Clone)]
#[serde(tag = "decision_type")]
pub enum Decision {
    /// Provide this store path
    Provide(ProvideData),
    /// Returns ENOENT
    Ignore,
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq, Clone)]
#[serde(tag = "resolution_type")]
pub enum Resolution {
    /// Constant resolution is always issued no matter the context.
    ConstantResolution(ResolutionData),
}

impl Resolution {
    pub fn requested_path(&self) -> &String {
        match self {
            Self::ConstantResolution(res_data) => &res_data.requested_path,
        }
    }
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq, Clone)]
pub struct ResolutionData {
    pub requested_path: String,
    pub decision: Decision,
}

// TODO: BTreeMap provide O(log n) search, do we need better?
pub type ResolutionDB = BTreeMap<String, Resolution>;

pub fn load_resolution_db(filename: PathBuf) -> ResolutionDB {
    serde_json::from_slice::<Vec<Resolution>>(
        &fs::read(filename).expect("Failed to read resolution DB"),
    )
    .expect("Failed to load resolution DB")
    .into_iter()
    .map(|resolution| (resolution.requested_path().clone(), resolution))
    .collect()
}

/// Unify two set of resolutions, right taking priority over left.
pub fn merge_resolution_db(left: ResolutionDB, right: ResolutionDB) -> ResolutionDB {
    left.into_iter().chain(right).collect()
}
