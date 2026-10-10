use crate::{HostId, HostSessionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Opaque authority to request one bounded read-only assessment. It never
/// authorizes a systemd mutation and is not accepted by mutation APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct SystemdStopImpactInspectionId(pub Uuid);

impl SystemdStopImpactInspectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SystemdStopImpactInspectionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactCompleteness {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactUncertainty {
    DirectOnly,
    ConditionalImpact,
    UnknownImpact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactProvenance {
    Conditional,
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactRelationship {
    RequiredBy,
    BoundBy,
    ConsistsOf,
    PropagatesStopTo,
    StopWhenUnneededCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactConditionalClassification {
    ConditionallyAffected,
    NotStopWhenUnneeded,
    RetainedByUnaffectedReference,
    CoverageUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactWarning {
    NodeLimit,
    EdgeLimit,
    DepthLimit,
    QueryLimit,
    TimeLimit,
    OutputLimit,
    AliasAmbiguity,
    UnsupportedProperty,
    UnsupportedEnumerant,
    DiagnosticLimit,
    ConcurrentTopologyChange,
    CandidateCoverageIncomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactDiagnosticKind {
    OnSuccessActivation,
    OnFailureActivation,
    Trigger,
    TriggeredBy,
    UpheldByReactivation,
    NonDefaultOnSuccessJobMode,
    NonDefaultOnFailureJobMode,
    SuccessManagerAction,
    FailureManagerAction,
    UnsupportedEnumerant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactJobMode {
    Fail,
    Replace,
    ReplaceIrreversibly,
    Isolate,
    Flush,
    IgnoreDependencies,
    IgnoreRequirements,
    Trigger,
    RestartDependencies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StopImpactManagerAction {
    None,
    Reboot,
    RebootForce,
    RebootImmediate,
    Poweroff,
    PoweroffForce,
    PoweroffImmediate,
    Exit,
    ExitForce,
    SoftReboot,
    SoftRebootForce,
    Kexec,
    KexecForce,
    Halt,
    HaltForce,
    HaltImmediate,
    Rescue,
    Emergency,
    FactoryReset,
}

/// Bounded, validated, passive systemd context. It carries no command,
/// environment, mutation target, or authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SystemdStopImpactDiagnostic {
    pub canonical_unit: String,
    pub kind: StopImpactDiagnosticKind,
    pub related_unit: Option<String>,
    pub job_mode: Option<StopImpactJobMode>,
    pub manager_action: Option<StopImpactManagerAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SystemdStopImpactUnit {
    pub canonical_unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub can_stop: bool,
    pub refuse_manual_stop: bool,
    pub stop_when_unneeded: bool,
    pub has_pending_job: bool,
    pub depth: u8,
    pub provenance: StopImpactProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SystemdStopImpactEdge {
    pub source: String,
    pub target: String,
    pub relationship: StopImpactRelationship,
    pub provenance: StopImpactProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SystemdStopImpactConditionalConsequence {
    pub canonical_unit: String,
    pub sources: Vec<String>,
    pub classification: StopImpactConditionalClassification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SystemdStopImpactAccounting {
    pub observed_unit_records: u32,
    pub observed_relationship_references: u32,
    pub observed_candidate_references: u32,
    pub retained_units: u32,
    pub retained_edges: u32,
    pub retained_candidates: u32,
    pub retained_diagnostics: u32,
    pub omitted_known_units: u32,
    pub omitted_known_edges: u32,
    pub omitted_known_candidates: u32,
    pub omitted_known_diagnostics: u32,
    pub unresolved_frontier_references: u32,
    pub actual_ssh_queries: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SystemdStopImpactAssessment {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub root_unit: String,
    pub observed_at: String,
    pub completeness: StopImpactCompleteness,
    pub uncertainty: StopImpactUncertainty,
    pub root_consistent: bool,
    pub units: Vec<SystemdStopImpactUnit>,
    pub edges: Vec<SystemdStopImpactEdge>,
    pub conditional_consequences: Vec<SystemdStopImpactConditionalConsequence>,
    pub conditional_diagnostics: Vec<SystemdStopImpactDiagnostic>,
    pub accounting: SystemdStopImpactAccounting,
    pub warnings: Vec<StopImpactWarning>,
    pub limitations: Vec<String>,
}
