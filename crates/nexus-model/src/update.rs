use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Opaque, one-shot identity for the exact native announcement authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct UpdateAnnouncementId(pub Uuid);

impl UpdateAnnouncementId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for UpdateAnnouncementId {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque, one-shot identity for exact native bytes and installer context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct VerifiedArtifactId(pub Uuid);

impl VerifiedArtifactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for VerifiedArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

/// Display-safe phase of the application-global native updater authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    Idle,
    Checking,
    UpToDate,
    UpdateAnnounced,
    Downloading,
    Verifying,
    Verified,
    Installing,
}

/// Narrow renderer projection. Download URLs, signatures and bytes remain native-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOperationSnapshot {
    pub current_version: String,
    pub phase: UpdatePhase,
    pub available_version: Option<String>,
    pub announcement_id: Option<UpdateAnnouncementId>,
    pub verified_artifact_id: Option<VerifiedArtifactId>,
    pub installation_supported: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_projection_contains_only_display_safe_fields() {
        let id = UpdateAnnouncementId(Uuid::nil());
        let announced = UpdateOperationSnapshot {
            current_version: "0.1.0".into(),
            phase: UpdatePhase::UpdateAnnounced,
            available_version: Some("0.2.0".into()),
            announcement_id: Some(id),
            verified_artifact_id: None,
            installation_supported: false,
        };
        assert_eq!(
            serde_json::to_value(announced).unwrap(),
            json!({
                "currentVersion": "0.1.0",
                "phase": "updateAnnounced",
                "availableVersion": "0.2.0",
                "announcementId": "00000000-0000-0000-0000-000000000000",
                "verifiedArtifactId": null,
                "installationSupported": false,
            })
        );

        let verified = UpdateOperationSnapshot {
            current_version: "0.1.0".into(),
            phase: UpdatePhase::Verified,
            available_version: Some("0.2.0".into()),
            announcement_id: None,
            verified_artifact_id: Some(VerifiedArtifactId(Uuid::nil())),
            installation_supported: true,
        };
        let value = serde_json::to_value(verified).unwrap();
        assert_eq!(value["phase"], "verified");
        assert_eq!(value["announcementId"], serde_json::Value::Null);
        assert_eq!(
            value["verifiedArtifactId"],
            "00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(value["installationSupported"], true);
        assert_eq!(value.as_object().unwrap().len(), 6);
    }
}
