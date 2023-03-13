use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct Popcount {
    pub build_inputs: HashMap<String, u32>,
    pub propagated_build_inputs: HashMap<String, u32>,
    pub native_build_inputs: HashMap<String, u32>,
    pub propagated_native_build_inputs: HashMap<String, u32>
}
