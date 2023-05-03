use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

use crate::cache::StorePath;
/// Resolution is data that enable the tool to automate a situation where
/// a manual decision has to be taken.

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize, Debug)]
pub struct ProvideData {
    pub kind: fuser::FileType,
    pub file_entry_name: String,
    pub store_path: StorePath,
}

impl ProvideData {
    pub fn to_human_toml_table(&self) -> toml::Table {
        let mut table = toml::Table::new();

        table.insert(
            "kind".into(),
            match self.kind {
                fuser::FileType::Socket => "socket",
                fuser::FileType::Symlink => "symlink",
                fuser::FileType::NamedPipe => "named-pipe",
                fuser::FileType::Directory => "directory",
                fuser::FileType::CharDevice => "char-device",
                fuser::FileType::BlockDevice => "block-device",
                fuser::FileType::RegularFile => "regular-file",
            }
            .into(),
        );
        table.insert(
            "file_entry_name".into(),
            self.file_entry_name.clone().into(),
        );
        table.insert(
            "store_path".into(),
            toml::Table::try_from(&self.store_path).unwrap().into(),
        );

        table
    }
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq, Clone, Debug)]
#[serde(tag = "decision")]
pub enum Decision {
    /// Provide this store path
    Provide(ProvideData),
    /// Returns ENOENT
    Ignore,
}

impl Decision {
    pub fn to_human_toml_table(&self) -> toml::Table {
        let mut table = toml::Table::new();

        if let Self::Provide(data) = self {
            table.insert("decision".into(), "provide".into());
            table.extend(data.to_human_toml_table());
        }

        table
    }
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq, Clone)]
#[serde(tag = "resolution")]
#[non_exhaustive]
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

    pub fn to_human_toml_table(&self) -> toml::Table {
        let mut gtable = toml::Table::new();

        let Self::ConstantResolution(data) = self;

        {
            let mut table = toml::Table::new();
            table.insert("resolution".into(), "constant".into());
            table.extend(data.decision.to_human_toml_table());
            gtable.insert(data.requested_path.clone(), table.into());
        }

        gtable
    }
}

#[derive(Serialize, Deserialize, Eq, Hash, PartialEq, Clone)]
pub struct ResolutionData {
    pub requested_path: String,
    pub decision: Decision,
}

// TODO: BTreeMap provide O(log n) search, do we need better?
pub type ResolutionDB = BTreeMap<String, Resolution>;

pub fn db_to_human_toml(db: &ResolutionDB) -> toml::Table {
    let mut table = toml::Table::new();

    for item in db.values() {
        table.extend(item.to_human_toml_table());
    }

    table
}

fn locate_resolution_db(search_path: PathBuf) -> Option<PathBuf> {
    None
}

/// Search in the provided path for a resolution database.
pub fn load_resolution_db(search_path: PathBuf) -> Option<ResolutionDB> {
    locate_resolution_db(search_path).and_then(|filename| {
        Some(
            serde_json::from_slice::<Vec<Resolution>>(
                &fs::read(filename).expect("Failed to read resolution DB"),
            )
            .expect("Failed to load resolution DB")
            .into_iter()
            .map(|resolution| (resolution.requested_path().clone(), resolution))
            .collect(),
        )
    })
}

/// Unify two set of resolutions, right taking priority over left.
pub fn merge_resolution_db(left: ResolutionDB, right: ResolutionDB) -> ResolutionDB {
    left.into_iter().chain(right).collect()
}
