use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Whether the configured release channel announces a newer version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UpdateCheckStatus {
    UpToDate,
    UpdateAnnounced,
}

/// A narrow projection of a manual update check. No manifest or download authority crosses IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckSnapshot {
    pub current_version: String,
    pub status: UpdateCheckStatus,
    pub available_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_projection_contains_only_the_reviewed_fields() {
        let current = UpdateCheckSnapshot {
            current_version: "0.1.0".into(),
            status: UpdateCheckStatus::UpToDate,
            available_version: None,
        };
        assert_eq!(
            serde_json::to_value(current).unwrap(),
            json!({
                "currentVersion": "0.1.0",
                "status": "upToDate",
                "availableVersion": null,
            })
        );

        let announced = UpdateCheckSnapshot {
            current_version: "0.1.0".into(),
            status: UpdateCheckStatus::UpdateAnnounced,
            available_version: Some("0.2.0".into()),
        };
        assert_eq!(
            serde_json::to_value(announced).unwrap(),
            json!({
                "currentVersion": "0.1.0",
                "status": "updateAnnounced",
                "availableVersion": "0.2.0",
            })
        );
    }
}
