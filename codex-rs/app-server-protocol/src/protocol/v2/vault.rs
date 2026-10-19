use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadVaultAttachParams {
    pub thread_id: String,
    /// App-server-local path to a broker socket and one-time grant ticket profile.
    pub profile: String,
    /// Explicit aliases matching the preissued grant, at most eight.
    pub scope: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadVaultStatusParams {
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadVaultDetachParams {
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadVaultStatusResponse {
    pub aliases: Vec<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadVaultAttachResponse {
    pub aliases: Vec<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadVaultDetachResponse {
    pub aliases: Vec<String>,
    pub expires_at: Option<i64>,
}
