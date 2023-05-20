use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};
use thiserror::Error;

use crate::cache::StorePath;

#[derive(Error, Debug)]
pub enum ParseResolutionError {
    #[error("missing field `{0}`")]
    MissingField(String),
    #[error("expected type `{0}` for field `{1}`")]
    UnexpectedType(String, String),
}

type ParseResult<T> = Result<T, ParseResolutionError>;

/// Resolution is data that enable the tool to automate a situation where
/// a manual decision has to be taken.

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize, Debug)]
pub struct ProvideData {
    pub kind: fuser::FileType,
    pub file_entry_name: String,
    pub store_path: StorePath,
}

fn parse_filetype_kind(v: &str) -> ParseResult<fuser::FileType> {
    Ok(match v {
        "socket" => fuser::FileType::Socket,
        "symlink" => fuser::FileType::Symlink,
        "named-pipe" => fuser::FileType::NamedPipe,
        "directory" => fuser::FileType::Directory,
        "char-device" => fuser::FileType::CharDevice,
        "block-device" => fuser::FileType::BlockDevice,
        "regular-file" => fuser::FileType::RegularFile,
        _ => {
            return Err(ParseResolutionError::UnexpectedType(
                "fuser::FileType".into(),
                "kind".into(),
            ))
        }
    })
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

    pub fn from_toml(mut data: toml::Table) -> ParseResult<Self> {
        Ok(ProvideData {
            kind: match data.get("kind") {
                Some(toml::Value::String(v)) => parse_filetype_kind(v)?,
                None => return Err(ParseResolutionError::MissingField("kind".into())),
                _ => {
                    return Err(ParseResolutionError::UnexpectedType(
                        "string".into(),
                        "kind".into(),
                    ))
                }
            },
            // use the deserializer here.
            file_entry_name: data
                .remove("file_entry_name")
                .map(|v| match v {
                    toml::Value::String(v) => Ok(v),
                    _ => Err(ParseResolutionError::UnexpectedType(
                        "string".into(),
                        "file_entry_name".into(),
                    )),
                })
                .ok_or_else(|| ParseResolutionError::MissingField("file_entry_name".into()))??,
            store_path: data.remove("store_path").expect("missing `store_path` field").try_into()
            .unwrap(),
        })
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

        match self {
            Self::Provide(data) => {
                table.insert("decision".into(), "provide".into());
                table.extend(data.to_human_toml_table());
            }
            Self::Ignore => {
                table.insert("decision".into(), "ignore".into());
            }
        }

        table
    }

    pub fn from_toml(decision: toml::Table) -> ParseResult<Self> {
        Ok(match decision.get("decision") {
            Some(toml::Value::String(decision_choice)) => match decision_choice.as_str() {
                "ignore" => Self::Ignore,
                "provide" => Self::Provide(ProvideData::from_toml(decision)?),
                _ => {
                    return Err(ParseResolutionError::UnexpectedType(
                        "`ignore` or `provide`".into(),
                        "decision".into(),
                    ))
                }
            },
            None => return Err(ParseResolutionError::MissingField("decision".into())),
            _ => {
                return Err(ParseResolutionError::UnexpectedType(
                    "`ignore` or `provide`".into(),
                    "decision".into(),
                ))
            }
        })
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

    pub fn from_toml_item(resolution: (String, toml::Value)) -> ParseResult<(String, Self)> {
        Ok((
            resolution.0.clone(),
            Self::ConstantResolution(ResolutionData {
                requested_path: resolution.0.clone(),
                decision: Decision::from_toml(match resolution.1 {
                    toml::Value::Table(table) => table,
                    _ => {
                        return Err(ParseResolutionError::UnexpectedType(
                            "a table".into(),
                            resolution.0,
                        ))
                    }
                })?,
            }),
        ))
    }

    pub fn from_toml(resolutions: toml::Value) -> ParseResult<ResolutionDB> {
        match resolutions {
            toml::Value::Table(resolutions_map) => Ok(resolutions_map
                .into_iter()
                .map(Self::from_toml_item)
                .collect::<ParseResult<ResolutionDB>>()?),
            _ => Err(ParseResolutionError::UnexpectedType(
                "an array of table".into(),
                "the whole document".into(),
            )),
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

pub fn read_resolution_db(data: &str) -> Option<ResolutionDB> {
    Resolution::from_toml(
        toml::from_str(data)
            .expect("Failed to parse the TOML"),
    )
    .ok()
}

/// Search in the provided path for a resolution database.
pub fn load_resolution_db(search_path: PathBuf) -> Option<ResolutionDB> {
    locate_resolution_db(search_path).and_then(|filename| read_resolution_db(&std::fs::read_to_string(filename).expect("Failed to read resolution DB from file")))
}

/// Unify two set of resolutions, right taking priority over left.
pub fn merge_resolution_db(left: ResolutionDB, right: ResolutionDB) -> ResolutionDB {
    left.into_iter().chain(right).collect()
}
