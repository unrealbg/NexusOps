use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, StopImpactCompleteness,
    StopImpactConditionalClassification, StopImpactDiagnosticKind, StopImpactJobMode,
    StopImpactManagerAction, StopImpactProvenance, StopImpactRelationship, StopImpactUncertainty,
    StopImpactWarning, SystemdStopImpactAccounting, SystemdStopImpactAssessment,
    SystemdStopImpactConditionalConsequence, SystemdStopImpactDiagnostic, SystemdStopImpactEdge,
    SystemdStopImpactUnit,
};
use nexus_operations::{RemoteSession, SystemdStopImpactEngine, SystemdStopImpactQuery};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    time::Duration,
};
use tokio::time::{Instant, timeout_at};
use tokio_util::sync::CancellationToken;

const MAX_NODES: usize = 64;
const MAX_EDGES: usize = 512;
const MAX_DIAGNOSTICS: usize = 512;
const MAX_DEPTH: u8 = 4;
const MAX_ALIASES: usize = 16;
const MAX_PROPERTY_BYTES: usize = 8 * 1024;
const MAX_BLOCK_BYTES: usize = 32 * 1024;
const MAX_AGGREGATE_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_WORKING_SET_BYTES: usize = 2 * 1024 * 1024;
// Reserve projection storage during traversal, before it is allocated in finish().
// Each owned slot includes the DTO, two <=255-byte identities and allocation
// overhead. Deduplication borrows at most one 8 KiB property's identities.
const DIAGNOSTIC_WORKING_SET_BYTES: usize =
    (MAX_DIAGNOSTICS + 1) * 1024 + MAX_PROPERTY_BYTES * size_of::<&str>();
const MAX_QUERIES: u8 = 6;
const FINAL_QUERY_RESERVE: Duration = Duration::from_secs(8);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

const DIRECT: [(&str, StopImpactRelationship); 4] = [
    ("RequiredBy", StopImpactRelationship::RequiredBy),
    ("BoundBy", StopImpactRelationship::BoundBy),
    ("ConsistsOf", StopImpactRelationship::ConsistsOf),
    ("PropagatesStopTo", StopImpactRelationship::PropagatesStopTo),
];
const CANDIDATE_FORWARD: [&str; 5] = ["Requires", "Wants", "Requisite", "BindsTo", "Upholds"];
const CANDIDATE_REVERSE: [&str; 5] = [
    "RequiredBy",
    "WantedBy",
    "RequisiteOf",
    "BoundBy",
    "UpheldBy",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedStopImpactUnit {
    pub id: String,
    pub names: Vec<String>,
    pub following: Option<String>,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub can_stop: bool,
    pub refuse_manual_stop: bool,
    pub has_pending_job: bool,
    pub need_daemon_reload: bool,
    pub stop_when_unneeded: bool,
    relationships: BTreeMap<String, Vec<String>>,
    on_success_job_mode: String,
    on_failure_job_mode: String,
    success_action: String,
    failure_action: String,
    unsupported_on_success_job_mode: bool,
    unsupported_on_failure_job_mode: bool,
    unsupported_success_action: bool,
    unsupported_failure_action: bool,
}

impl ParsedStopImpactUnit {
    fn related(&self, property: &str) -> &[String] {
        self.relationships
            .get(property)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn conditional_effects_present(&self) -> bool {
        !self.related("OnSuccess").is_empty()
            || !self.related("OnFailure").is_empty()
            || !self.related("Triggers").is_empty()
            || !self.related("TriggeredBy").is_empty()
            || !self.related("UpheldBy").is_empty()
            || self.on_success_job_mode != "fail"
            || self.on_failure_job_mode != "replace"
            || self.success_action != "none"
            || self.failure_action != "none"
    }

    fn has_unsupported_enumerant(&self) -> bool {
        self.unsupported_on_success_job_mode
            || self.unsupported_on_failure_job_mode
            || self.unsupported_success_action
            || self.unsupported_failure_action
    }

    fn conditional_diagnostics(&self) -> impl Iterator<Item = DiagnosticView<'_>> {
        let relationships = [
            ("OnSuccess", StopImpactDiagnosticKind::OnSuccessActivation),
            ("OnFailure", StopImpactDiagnosticKind::OnFailureActivation),
            ("Triggers", StopImpactDiagnosticKind::Trigger),
            ("TriggeredBy", StopImpactDiagnosticKind::TriggeredBy),
            ("UpheldBy", StopImpactDiagnosticKind::UpheldByReactivation),
        ]
        .into_iter()
        .flat_map(move |(property, kind)| {
            self.related(property)
                .iter()
                .map(move |related_unit| DiagnosticView {
                    canonical_unit: &self.id,
                    kind,
                    related_unit: Some(related_unit),
                    job_mode: None,
                    manager_action: None,
                })
        });
        let mut scalars = [None; 5];
        if self.has_unsupported_enumerant() {
            scalars[0] = Some(DiagnosticView {
                canonical_unit: &self.id,
                kind: StopImpactDiagnosticKind::UnsupportedEnumerant,
                related_unit: None,
                job_mode: None,
                manager_action: None,
            });
        }
        for (index, (value, default, unsupported, kind)) in [
            (
                self.on_success_job_mode.as_str(),
                "fail",
                self.unsupported_on_success_job_mode,
                StopImpactDiagnosticKind::NonDefaultOnSuccessJobMode,
            ),
            (
                self.on_failure_job_mode.as_str(),
                "replace",
                self.unsupported_on_failure_job_mode,
                StopImpactDiagnosticKind::NonDefaultOnFailureJobMode,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if !unsupported && value != default {
                scalars[index + 1] = Some(DiagnosticView {
                    canonical_unit: &self.id,
                    kind,
                    related_unit: None,
                    job_mode: stop_impact_job_mode(value),
                    manager_action: None,
                });
            }
        }
        for (index, (value, unsupported, kind)) in [
            (
                self.success_action.as_str(),
                self.unsupported_success_action,
                StopImpactDiagnosticKind::SuccessManagerAction,
            ),
            (
                self.failure_action.as_str(),
                self.unsupported_failure_action,
                StopImpactDiagnosticKind::FailureManagerAction,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if !unsupported && value != "none" {
                scalars[index + 3] = Some(DiagnosticView {
                    canonical_unit: &self.id,
                    kind,
                    related_unit: None,
                    job_mode: None,
                    manager_action: stop_impact_manager_action(value),
                });
            }
        }
        relationships.chain(scalars.into_iter().flatten())
    }
}

#[derive(Clone, Copy)]
struct DiagnosticView<'a> {
    canonical_unit: &'a str,
    kind: StopImpactDiagnosticKind,
    related_unit: Option<&'a str>,
    job_mode: Option<StopImpactJobMode>,
    manager_action: Option<StopImpactManagerAction>,
}

impl DiagnosticView<'_> {
    fn key(&self) -> (&str, u8, &str) {
        (
            self.canonical_unit,
            diagnostic_code(self.kind),
            self.related_unit.unwrap_or_default(),
        )
    }

    fn into_owned(self) -> SystemdStopImpactDiagnostic {
        SystemdStopImpactDiagnostic {
            canonical_unit: self.canonical_unit.to_owned(),
            kind: self.kind,
            related_unit: self.related_unit.map(str::to_owned),
            job_mode: self.job_mode,
            manager_action: self.manager_action,
        }
    }
}

struct DiagnosticProjection<'a> {
    retained: Vec<SystemdStopImpactDiagnostic>,
    // Reset between properties/records. These references borrow the existing
    // parsed records or alias map; omitted identities are never cloned.
    seen_related: Vec<&'a str>,
    unique_count: usize,
    #[cfg(test)]
    peak_retained: usize,
    #[cfg(test)]
    peak_seen_related: usize,
}

impl<'a> DiagnosticProjection<'a> {
    fn new() -> Self {
        Self {
            retained: Vec::with_capacity(MAX_DIAGNOSTICS),
            seen_related: Vec::with_capacity(MAX_PROPERTY_BYTES),
            unique_count: 0,
            #[cfg(test)]
            peak_retained: 0,
            #[cfg(test)]
            peak_seen_related: 0,
        }
    }

    fn project_record(
        &mut self,
        record: &'a ParsedStopImpactUnit,
        aliases: &'a HashMap<String, String>,
        ambiguous_aliases: &HashSet<String>,
    ) {
        self.seen_related.clear();
        let mut previous_kind = None;
        for mut diagnostic in record.conditional_diagnostics() {
            if previous_kind != Some(diagnostic.kind) {
                self.seen_related.clear();
                previous_kind = Some(diagnostic.kind);
            }
            if let Some(related) = diagnostic.related_unit {
                let normalized = if ambiguous_aliases.contains(related) {
                    related
                } else {
                    aliases.get(related).map_or(related, String::as_str)
                };
                let Err(index) = self.seen_related.binary_search(&normalized) else {
                    continue;
                };
                // Strict parsing bounds each property's reference count by its
                // byte limit. Insertions cannot grow the reserved allocation.
                assert!(self.seen_related.len() < MAX_PROPERTY_BYTES);
                self.seen_related.insert(index, normalized);
                diagnostic.related_unit = Some(normalized);
            }
            self.unique_count += 1;
            let index = self
                .retained
                .binary_search_by(|retained| {
                    (
                        retained.canonical_unit.as_str(),
                        diagnostic_code(retained.kind),
                        retained.related_unit.as_deref().unwrap_or_default(),
                    )
                        .cmp(&diagnostic.key())
                })
                .expect_err("canonical records and normalized per-kind identities are unique");
            if index < MAX_DIAGNOSTICS {
                // Evict before allocating, so even intermediate owned storage
                // never contains 513 diagnostics or duplicated sort keys.
                if self.retained.len() == MAX_DIAGNOSTICS {
                    self.retained.pop();
                }
                self.retained.insert(index, diagnostic.into_owned());
            }
            #[cfg(test)]
            {
                self.peak_retained = self.peak_retained.max(self.retained.len());
                self.peak_seen_related = self.peak_seen_related.max(self.seen_related.len());
                assert!(self.retained.len() <= MAX_DIAGNOSTICS);
                assert_eq!(self.retained.capacity(), MAX_DIAGNOSTICS);
                assert_eq!(self.seen_related.capacity(), MAX_PROPERTY_BYTES);
            }
        }
    }
}

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned invalid systemd stop-impact data.",
    )
}

/// Strictly parses one or more `systemctl show --all` blocks from the fixed
/// 37-property contract. Unknown, duplicate, missing, oversized, or malformed
/// properties fail the whole response closed.
pub fn parse_systemd_stop_impact(output: &str) -> Result<Vec<ParsedStopImpactUnit>, AppError> {
    if output.is_empty() || output.contains('\0') {
        return Err(invalid());
    }
    let normalized = output.strip_suffix('\n').unwrap_or(output);
    let mut result = Vec::new();
    for block in normalized.split("\n\n") {
        if block.is_empty() || block.len() > MAX_BLOCK_BYTES {
            return Err(invalid());
        }
        let mut fields = BTreeMap::new();
        for line in block.split('\n') {
            let (key, value) = line.split_once('=').ok_or_else(invalid)?;
            if value.len() > MAX_PROPERTY_BYTES
                || !nexus_operations::SYSTEMD_STOP_IMPACT_PROPERTIES.contains(&key)
                || fields.insert(key, value).is_some()
            {
                return Err(invalid());
            }
        }
        if fields.len() != nexus_operations::SYSTEMD_STOP_IMPACT_PROPERTIES.len()
            || nexus_operations::SYSTEMD_STOP_IMPACT_PROPERTIES
                .iter()
                .any(|key| !fields.contains_key(key))
        {
            return Err(invalid());
        }
        let observed_id = unit(fields["Id"])?;
        let mut names = unit_list(fields["Names"])?;
        if names.is_empty()
            || names.len() > MAX_ALIASES
            || !names.iter().any(|name| name == &observed_id)
        {
            return Err(invalid());
        }
        let following = if fields["Following"].is_empty() {
            None
        } else {
            Some(unit(fields["Following"])?)
        };
        let id = following.clone().unwrap_or(observed_id);
        if !names.contains(&id) {
            if names.len() == MAX_ALIASES {
                return Err(invalid());
            }
            names.push(id.clone());
        }
        let relationships = [
            "Requires",
            "RequiredBy",
            "Requisite",
            "RequisiteOf",
            "Wants",
            "WantedBy",
            "BindsTo",
            "BoundBy",
            "PartOf",
            "ConsistsOf",
            "PropagatesStopTo",
            "StopPropagatedFrom",
            "Upholds",
            "UpheldBy",
            "Conflicts",
            "ConflictedBy",
            "Before",
            "After",
            "Triggers",
            "TriggeredBy",
            "OnSuccess",
            "OnFailure",
        ]
        .into_iter()
        .map(|key| Ok((key.to_owned(), unit_list(fields[key])?)))
        .collect::<Result<BTreeMap<_, _>, AppError>>()?;
        let on_success_job_mode = enumerant(fields["OnSuccessJobMode"], JOB_MODES)?;
        let on_failure_job_mode = enumerant(fields["OnFailureJobMode"], JOB_MODES)?;
        let success_action = enumerant(fields["SuccessAction"], MANAGER_ACTIONS)?;
        let failure_action = enumerant(fields["FailureAction"], MANAGER_ACTIONS)?;
        result.push(ParsedStopImpactUnit {
            id,
            names,
            following,
            load_state: token(fields["LoadState"], 64)?,
            active_state: token(fields["ActiveState"], 64)?,
            sub_state: token(fields["SubState"], 64)?,
            can_stop: yes_no(fields["CanStop"])?,
            refuse_manual_stop: yes_no(fields["RefuseManualStop"])?,
            has_pending_job: job(fields["Job"])?,
            need_daemon_reload: yes_no(fields["NeedDaemonReload"])?,
            stop_when_unneeded: yes_no(fields["StopWhenUnneeded"])?,
            relationships,
            on_success_job_mode: on_success_job_mode.0,
            on_failure_job_mode: on_failure_job_mode.0,
            success_action: success_action.0,
            failure_action: failure_action.0,
            unsupported_on_success_job_mode: on_success_job_mode.1,
            unsupported_on_failure_job_mode: on_failure_job_mode.1,
            unsupported_success_action: success_action.1,
            unsupported_failure_action: failure_action.1,
        });
    }
    Ok(result)
}

fn unit(value: &str) -> Result<String, AppError> {
    SystemdStopImpactQuery::single(value.to_owned()).map_err(|_| invalid())?;
    Ok(value.to_owned())
}

fn token(value: &str, maximum: usize) -> Result<String, AppError> {
    if value.is_empty()
        || value.len() > maximum
        || value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err(invalid());
    }
    Ok(value.to_owned())
}

fn yes_no(value: &str) -> Result<bool, AppError> {
    match value {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(invalid()),
    }
}

fn job(value: &str) -> Result<bool, AppError> {
    if value.is_empty() {
        return Ok(false);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    value
        .parse::<u32>()
        .ok()
        .and_then(std::num::NonZeroU32::new)
        .map(|_| true)
        .ok_or_else(invalid)
}

const JOB_MODES: &[&str] = &[
    "fail",
    "replace",
    "replace-irreversibly",
    "isolate",
    "flush",
    "ignore-dependencies",
    "ignore-requirements",
    "trigger",
    "restart-dependencies",
];

const MANAGER_ACTIONS: &[&str] = &[
    "none",
    "reboot",
    "reboot-force",
    "reboot-immediate",
    "poweroff",
    "poweroff-force",
    "poweroff-immediate",
    "exit",
    "exit-force",
    "soft-reboot",
    "soft-reboot-force",
    "kexec",
    "kexec-force",
    "halt",
    "halt-force",
    "halt-immediate",
    "rescue",
    "emergency",
    "factory-reset",
];

fn stop_impact_job_mode(value: &str) -> Option<StopImpactJobMode> {
    Some(match value {
        "fail" => StopImpactJobMode::Fail,
        "replace" => StopImpactJobMode::Replace,
        "replace-irreversibly" => StopImpactJobMode::ReplaceIrreversibly,
        "isolate" => StopImpactJobMode::Isolate,
        "flush" => StopImpactJobMode::Flush,
        "ignore-dependencies" => StopImpactJobMode::IgnoreDependencies,
        "ignore-requirements" => StopImpactJobMode::IgnoreRequirements,
        "trigger" => StopImpactJobMode::Trigger,
        "restart-dependencies" => StopImpactJobMode::RestartDependencies,
        _ => return None,
    })
}

fn stop_impact_manager_action(value: &str) -> Option<StopImpactManagerAction> {
    Some(match value {
        "none" => StopImpactManagerAction::None,
        "reboot" => StopImpactManagerAction::Reboot,
        "reboot-force" => StopImpactManagerAction::RebootForce,
        "reboot-immediate" => StopImpactManagerAction::RebootImmediate,
        "poweroff" => StopImpactManagerAction::Poweroff,
        "poweroff-force" => StopImpactManagerAction::PoweroffForce,
        "poweroff-immediate" => StopImpactManagerAction::PoweroffImmediate,
        "exit" => StopImpactManagerAction::Exit,
        "exit-force" => StopImpactManagerAction::ExitForce,
        "soft-reboot" => StopImpactManagerAction::SoftReboot,
        "soft-reboot-force" => StopImpactManagerAction::SoftRebootForce,
        "kexec" => StopImpactManagerAction::Kexec,
        "kexec-force" => StopImpactManagerAction::KexecForce,
        "halt" => StopImpactManagerAction::Halt,
        "halt-force" => StopImpactManagerAction::HaltForce,
        "halt-immediate" => StopImpactManagerAction::HaltImmediate,
        "rescue" => StopImpactManagerAction::Rescue,
        "emergency" => StopImpactManagerAction::Emergency,
        "factory-reset" => StopImpactManagerAction::FactoryReset,
        _ => return None,
    })
}

fn enumerant(value: &str, supported: &[&str]) -> Result<(String, bool), AppError> {
    let value = token(value, 64)?;
    let unsupported = !supported.contains(&value.as_str());
    Ok((value, unsupported))
}

fn unit_list(value: &str) -> Result<Vec<String>, AppError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut seen = HashSet::new();
    shell_words(value)?
        .into_iter()
        .map(|value| {
            let value = unit(&value)?;
            if !seen.insert(value.clone()) {
                return Err(invalid());
            }
            Ok(value)
        })
        .collect()
}

fn shell_words(value: &str) -> Result<Vec<String>, AppError> {
    // systemd v255's string-array printer calls shell_maybe_quote(str, 0):
    // https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/src/shared/bus-print-properties.c#L228
    // Its default double-quoted form escapes SHELL_NEED_ESCAPE ("\\`$).
    // Decode that one CLI layer, never the canonical unit-name \xHH layer.
    // Retain literal unquoted/single-quoted identities for existing inputs.
    if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(invalid());
    }
    let mut words = Vec::new();
    let mut characters = value.chars().peekable();
    while characters.peek().is_some() {
        if characters.peek() == Some(&' ') {
            characters.next();
            continue;
        }
        let quote = match characters.peek() {
            Some('\'' | '"') => characters.next(),
            _ => None,
        };
        let mut closed = quote.is_none();
        let mut current = String::new();
        while let Some(character) = characters.next() {
            match (quote, character) {
                (Some(delimiter), character) if character == delimiter => {
                    closed = true;
                    break;
                }
                (None, ' ') => break,
                (None, '\'' | '"') => return Err(invalid()),
                (None | Some('"'), '\\') => {
                    let escaped = characters.next().ok_or_else(invalid)?;
                    match (quote, escaped) {
                        (Some('"'), '\\' | '"' | '`' | '$') => current.push(escaped),
                        (None, 'x') => {
                            // Already-canonical unquoted identity: typed unit
                            // validation checks both hex digits afterwards.
                            current.push('\\');
                            current.push('x');
                        }
                        _ => return Err(invalid()),
                    }
                }
                (_, character) => current.push(character),
            }
        }
        // Upstream emits one complete quoted token, never shell concatenation.
        if !closed
            || current.is_empty()
            || (quote.is_some() && characters.peek().is_some_and(|next| *next != ' '))
        {
            return Err(invalid());
        }
        words.push(current);
    }
    Ok(words)
}

#[derive(Clone)]
struct Pending {
    requested: String,
    depth: u8,
    provenance: StopImpactProvenance,
    candidate: bool,
    candidate_sources: BTreeSet<String>,
    relationship_order: u8,
}

#[derive(Clone)]
struct Retained {
    record: ParsedStopImpactUnit,
    depth: u8,
    provenance: StopImpactProvenance,
    candidate: bool,
}

#[derive(Clone)]
struct CandidateObservation {
    record: ParsedStopImpactUnit,
    depth: u8,
    sources: BTreeSet<String>,
}

struct Builder {
    host_id: HostId,
    host_session_id: HostSessionId,
    root_initial: ParsedStopImpactUnit,
    records: HashMap<String, Retained>,
    candidates: HashMap<String, CandidateObservation>,
    conditional_consequences:
        BTreeMap<String, (BTreeSet<String>, StopImpactConditionalClassification)>,
    aliases: HashMap<String, String>,
    ambiguous_aliases: HashSet<String>,
    queried: HashSet<String>,
    pending: VecDeque<Pending>,
    edges: BTreeMap<(String, String, u8), SystemdStopImpactEdge>,
    warnings: BTreeSet<u8>,
    accounting: SystemdStopImpactAccounting,
    incomplete: bool,
    conditional: bool,
    unknown: bool,
    aggregate_output_bytes: usize,
}

impl Builder {
    fn new(
        host_id: HostId,
        host_session_id: HostSessionId,
        root_requested: String,
        root: ParsedStopImpactUnit,
        initial_output_bytes: usize,
    ) -> Self {
        let mut aliases = HashMap::new();
        for name in &root.names {
            aliases.insert(name.clone(), root.id.clone());
        }
        let mut queried = HashSet::new();
        queried.insert(root_requested.clone());
        queried.extend(root.names.iter().cloned());
        let mut records = HashMap::new();
        records.insert(
            root.id.clone(),
            Retained {
                record: root.clone(),
                depth: 0,
                provenance: StopImpactProvenance::Direct,
                candidate: false,
            },
        );
        Self {
            host_id,
            host_session_id,
            root_initial: root,
            records,
            candidates: HashMap::new(),
            conditional_consequences: BTreeMap::new(),
            aliases,
            ambiguous_aliases: HashSet::new(),
            queried,
            pending: VecDeque::new(),
            edges: BTreeMap::new(),
            warnings: BTreeSet::new(),
            accounting: SystemdStopImpactAccounting {
                observed_unit_records: 1,
                observed_relationship_references: 0,
                observed_candidate_references: 0,
                retained_units: 1,
                retained_edges: 0,
                retained_candidates: 0,
                retained_diagnostics: 0,
                omitted_known_units: 0,
                omitted_known_edges: 0,
                omitted_known_candidates: 0,
                omitted_known_diagnostics: 0,
                unresolved_frontier_references: 0,
                actual_ssh_queries: 1,
            },
            incomplete: false,
            conditional: false,
            unknown: false,
            aggregate_output_bytes: initial_output_bytes,
        }
    }

    fn warning(&mut self, warning: StopImpactWarning) {
        self.warnings.insert(warning_code(warning));
    }

    fn working_set_bytes(&self) -> usize {
        let record_bytes =
            self.records.values().fold(0usize, |total, retained| {
                let relationships = retained.record.relationships.iter().fold(
                    0usize,
                    |subtotal, (property, units)| {
                        subtotal
                            .saturating_add(property.len())
                            .saturating_add(units.iter().map(String::len).sum::<usize>())
                            .saturating_add(units.len().saturating_mul(32))
                    },
                );
                total
                    .saturating_add(1024)
                    .saturating_add(retained.record.id.len())
                    .saturating_add(retained.record.names.iter().map(String::len).sum::<usize>())
                    .saturating_add(retained.record.following.as_ref().map_or(0, String::len))
                    .saturating_add(retained.record.load_state.len())
                    .saturating_add(retained.record.active_state.len())
                    .saturating_add(retained.record.sub_state.len())
                    .saturating_add(relationships)
            });
        let edge_bytes = self.edges.values().fold(0usize, |total, edge| {
            total
                .saturating_add(512)
                .saturating_add(edge.source.len())
                .saturating_add(edge.target.len())
        });
        let candidate_bytes = self.candidates.values().fold(0usize, |total, candidate| {
            total
                .saturating_add(1024)
                .saturating_add(candidate.record.id.len())
                .saturating_add(
                    candidate
                        .record
                        .relationships
                        .values()
                        .flatten()
                        .map(String::len)
                        .sum::<usize>(),
                )
                .saturating_add(candidate.sources.iter().map(String::len).sum::<usize>())
        });
        let alias_bytes = self
            .aliases
            .iter()
            .fold(0usize, |total, (alias, canonical)| {
                total
                    .saturating_add(256)
                    .saturating_add(alias.len())
                    .saturating_add(canonical.len())
            });
        let pending_bytes = self.pending.iter().fold(0usize, |total, pending| {
            total
                .saturating_add(512)
                .saturating_add(pending.requested.len())
                .saturating_add(
                    pending
                        .candidate_sources
                        .iter()
                        .map(String::len)
                        .sum::<usize>(),
                )
        });
        64usize
            .saturating_mul(1024)
            .saturating_add(record_bytes)
            .saturating_add(candidate_bytes)
            .saturating_add(edge_bytes)
            .saturating_add(alias_bytes)
            .saturating_add(pending_bytes)
            .saturating_add(DIAGNOSTIC_WORKING_SET_BYTES)
    }

    fn enforce_working_set(&mut self) -> bool {
        if self.working_set_bytes() <= MAX_WORKING_SET_BYTES {
            return true;
        }
        self.accounting.unresolved_frontier_references = self
            .accounting
            .unresolved_frontier_references
            .saturating_add(u32::try_from(self.pending.len()).unwrap_or(u32::MAX));
        self.pending.clear();
        self.incomplete = true;
        self.unknown = true;
        self.warning(StopImpactWarning::OutputLimit);
        false
    }

    fn inspect_record(&mut self, canonical: &str) {
        let Some(retained) = self.records.get(canonical).cloned() else {
            return;
        };
        if retained.record.need_daemon_reload
            || retained.record.has_pending_job
            || retained.record.conditional_effects_present()
        {
            self.unknown = true;
        }
        if retained.record.has_unsupported_enumerant() {
            self.incomplete = true;
            self.unknown = true;
            self.warning(StopImpactWarning::UnsupportedEnumerant);
        }
        self.accounting.observed_relationship_references = self
            .accounting
            .observed_relationship_references
            .saturating_add(
                retained
                    .record
                    .relationships
                    .values()
                    .map(|units| u32::try_from(units.len()).unwrap_or(u32::MAX))
                    .fold(0, u32::saturating_add),
            );
        for (property, relationship) in DIRECT {
            for target in retained.record.related(property) {
                self.admit_edge_and_query(
                    canonical,
                    target,
                    relationship,
                    retained.depth.saturating_add(1),
                    retained.provenance,
                    false,
                );
            }
        }
        if !retained.candidate || retained.record.stop_when_unneeded {
            for (candidate_index, property) in CANDIDATE_FORWARD.into_iter().enumerate() {
                for target in retained.record.related(property) {
                    self.accounting.observed_candidate_references = self
                        .accounting
                        .observed_candidate_references
                        .saturating_add(1);
                    self.admit_query(
                        target,
                        retained.depth.saturating_add(1),
                        StopImpactProvenance::Conditional,
                        true,
                        Some(canonical),
                        5 + u8::try_from(candidate_index).unwrap_or(u8::MAX),
                    );
                }
            }
        }
    }

    fn admit_edge_and_query(
        &mut self,
        source: &str,
        target: &str,
        relationship: StopImpactRelationship,
        depth: u8,
        provenance: StopImpactProvenance,
        candidate: bool,
    ) {
        if self.ambiguous_aliases.contains(target) {
            self.accounting.omitted_known_edges =
                self.accounting.omitted_known_edges.saturating_add(1);
            self.incomplete = true;
            self.unknown = true;
            self.warning(StopImpactWarning::AliasAmbiguity);
            self.admit_query(
                target,
                depth,
                provenance,
                candidate,
                None,
                relationship_code(relationship),
            );
            return;
        }
        let canonical_target = self
            .aliases
            .get(target)
            .cloned()
            .unwrap_or_else(|| target.to_owned());
        if self.edges.len() >= MAX_EDGES {
            self.accounting.omitted_known_edges =
                self.accounting.omitted_known_edges.saturating_add(1);
            self.incomplete = true;
            self.warning(StopImpactWarning::EdgeLimit);
        } else {
            let key = (
                source.to_owned(),
                canonical_target.clone(),
                relationship_code(relationship),
            );
            self.edges.entry(key).or_insert(SystemdStopImpactEdge {
                source: source.to_owned(),
                target: canonical_target.clone(),
                relationship,
                provenance,
            });
        }
        self.admit_query(
            &canonical_target,
            depth,
            provenance,
            candidate,
            None,
            relationship_code(relationship),
        );
    }

    fn admit_query(
        &mut self,
        requested: &str,
        depth: u8,
        provenance: StopImpactProvenance,
        candidate: bool,
        candidate_source: Option<&str>,
        relationship_order: u8,
    ) {
        if depth > MAX_DEPTH {
            self.accounting.unresolved_frontier_references = self
                .accounting
                .unresolved_frontier_references
                .saturating_add(1);
            self.incomplete = true;
            self.warning(StopImpactWarning::DepthLimit);
            if candidate {
                self.unknown = true;
                self.warning(StopImpactWarning::CandidateCoverageIncomplete);
            }
            return;
        }
        if self.ambiguous_aliases.contains(requested) {
            self.accounting.unresolved_frontier_references = self
                .accounting
                .unresolved_frontier_references
                .saturating_add(1);
            self.incomplete = true;
            self.unknown = true;
            self.warning(StopImpactWarning::AliasAmbiguity);
            return;
        }
        if let Some(canonical) = self.aliases.get(requested).cloned() {
            self.promote(&canonical, depth, provenance, candidate);
            return;
        }
        if self.queried.contains(requested) {
            return;
        }
        if let Some(existing) = self
            .pending
            .iter_mut()
            .find(|item| item.requested == requested)
        {
            if provenance > existing.provenance {
                existing.provenance = provenance;
            }
            if depth < existing.depth {
                existing.depth = depth;
                existing.relationship_order = relationship_order;
            } else if depth == existing.depth {
                existing.relationship_order = existing.relationship_order.min(relationship_order);
            }
            existing.candidate &= candidate;
            if let Some(source) = candidate_source {
                existing.candidate_sources.insert(source.to_owned());
            }
            return;
        }
        let mut candidate_sources = BTreeSet::new();
        if let Some(source) = candidate_source {
            candidate_sources.insert(source.to_owned());
        }
        self.pending.push_back(Pending {
            requested: requested.to_owned(),
            depth,
            provenance,
            candidate,
            candidate_sources,
            relationship_order,
        });
    }

    fn promote(
        &mut self,
        canonical: &str,
        depth: u8,
        provenance: StopImpactProvenance,
        candidate: bool,
    ) {
        let mut needs_propagation = false;
        let mut needs_inspection = false;
        if !self.records.contains_key(canonical)
            && let Some(candidate_observation) = self.candidates.remove(canonical)
        {
            self.records.insert(
                canonical.to_owned(),
                Retained {
                    record: candidate_observation.record,
                    depth: candidate_observation.depth.min(depth),
                    provenance,
                    candidate: false,
                },
            );
            needs_propagation = true;
            needs_inspection = true;
        }
        if let Some(existing) = self.records.get_mut(canonical) {
            if provenance > existing.provenance {
                existing.provenance = provenance;
                existing.candidate = false;
                needs_propagation = true;
            }
            existing.depth = existing.depth.min(depth);
            if !candidate {
                existing.candidate = false;
            }
        }
        if needs_propagation {
            self.promote_cached_descendants(canonical);
        }
        if needs_inspection {
            self.inspect_record(canonical);
        }
    }

    fn promote_cached_descendants(&mut self, start: &str) {
        let mut queue = VecDeque::from([start.to_owned()]);
        let mut seen = HashSet::new();
        while let Some(source) = queue.pop_front() {
            if !seen.insert(source.clone()) {
                continue;
            }
            let targets = self
                .edges
                .values_mut()
                .filter(|edge| {
                    edge.source == source
                        && edge.provenance == StopImpactProvenance::Conditional
                        && edge.relationship != StopImpactRelationship::StopWhenUnneededCandidate
                })
                .map(|edge| {
                    edge.provenance = StopImpactProvenance::Direct;
                    edge.target.clone()
                })
                .collect::<Vec<_>>();
            for target in targets {
                if let Some(record) = self.records.get_mut(&target)
                    && record.provenance == StopImpactProvenance::Conditional
                {
                    record.provenance = StopImpactProvenance::Direct;
                    record.candidate = false;
                    queue.push_back(target);
                }
            }
        }
    }

    fn take_batch(&mut self) -> Vec<Pending> {
        let priority = if self
            .pending
            .iter()
            .any(|pending| pending.provenance == StopImpactProvenance::Direct)
        {
            StopImpactProvenance::Direct
        } else {
            StopImpactProvenance::Conditional
        };
        let mut eligible = Vec::new();
        let mut deferred = VecDeque::new();
        while let Some(pending) = self.pending.pop_front() {
            if pending.provenance == priority {
                eligible.push(pending);
            } else {
                deferred.push_back(pending);
            }
        }
        eligible.sort_by(|left, right| {
            let left_name = self.aliases.get(&left.requested).unwrap_or(&left.requested);
            let right_name = self
                .aliases
                .get(&right.requested)
                .unwrap_or(&right.requested);
            (left.depth, left_name, left.relationship_order).cmp(&(
                right.depth,
                right_name,
                right.relationship_order,
            ))
        });
        let remainder = eligible.split_off(eligible.len().min(16));
        self.pending.extend(remainder);
        self.pending.extend(deferred);
        eligible
    }

    fn retain_batch(
        &mut self,
        requested: Vec<Pending>,
        parsed: Vec<ParsedStopImpactUnit>,
    ) -> Result<(), AppError> {
        let mut parsed = match_batch_by_identity(&requested, parsed)?;
        let mut inspect = Vec::new();
        for pending in requested {
            let record = parsed.remove(&pending.requested).ok_or_else(invalid)?;
            self.queried.insert(pending.requested.clone());
            self.accounting.observed_unit_records =
                self.accounting.observed_unit_records.saturating_add(1);
            if record.id != pending.requested && !record.names.contains(&pending.requested) {
                return Err(invalid());
            }
            let canonical = record.id.clone();
            self.canonicalize_edges(&pending.requested, &canonical);
            for name in &record.names {
                if let Some(previous) = self.aliases.get(name) {
                    if previous != &canonical {
                        self.aliases.remove(name);
                        self.ambiguous_aliases.insert(name.clone());
                        self.incomplete = true;
                        self.unknown = true;
                        self.warning(StopImpactWarning::AliasAmbiguity);
                    }
                } else if !self.ambiguous_aliases.contains(name) {
                    self.aliases.insert(name.clone(), canonical.clone());
                }
                self.queried.insert(name.clone());
            }
            if let Some(existing) = self.records.get(&canonical) {
                if existing.record != record {
                    self.incomplete = true;
                    self.unknown = true;
                    self.warning(StopImpactWarning::AliasAmbiguity);
                }
                self.promote(
                    &canonical,
                    pending.depth,
                    pending.provenance,
                    pending.candidate,
                );
                continue;
            }
            if let Some(existing) = self.candidates.get_mut(&canonical) {
                if existing.record != record {
                    self.incomplete = true;
                    self.unknown = true;
                    self.warning(StopImpactWarning::AliasAmbiguity);
                    continue;
                }
                existing.depth = existing.depth.min(pending.depth);
                existing.sources.extend(pending.candidate_sources);
                if pending.provenance == StopImpactProvenance::Direct || !pending.candidate {
                    self.promote(
                        &canonical,
                        pending.depth,
                        StopImpactProvenance::Direct,
                        false,
                    );
                }
                continue;
            }
            if self.records.len().saturating_add(self.candidates.len()) >= MAX_NODES {
                self.accounting.omitted_known_units =
                    self.accounting.omitted_known_units.saturating_add(1);
                if pending.candidate {
                    self.accounting.omitted_known_candidates =
                        self.accounting.omitted_known_candidates.saturating_add(1);
                    self.unknown = true;
                    self.warning(StopImpactWarning::CandidateCoverageIncomplete);
                }
                self.incomplete = true;
                self.warning(StopImpactWarning::NodeLimit);
                continue;
            }
            if pending.candidate && pending.provenance == StopImpactProvenance::Conditional {
                self.candidates.insert(
                    canonical,
                    CandidateObservation {
                        record,
                        depth: pending.depth,
                        sources: pending.candidate_sources,
                    },
                );
                continue;
            }
            self.records.insert(
                canonical.clone(),
                Retained {
                    record,
                    depth: pending.depth,
                    provenance: pending.provenance,
                    candidate: pending.candidate,
                },
            );
            inspect.push(canonical);
        }
        for canonical in inspect {
            self.inspect_record(&canonical);
        }
        if !self.has_direct_frontier() {
            self.promote_eligible_candidates();
        }
        Ok(())
    }

    fn has_direct_frontier(&self) -> bool {
        self.pending
            .iter()
            .any(|pending| pending.provenance == StopImpactProvenance::Direct)
    }

    fn promote_eligible_candidates(&mut self) {
        loop {
            let eligible = self
                .candidates
                .iter()
                .filter(|(_, candidate)| {
                    candidate_affected(
                        &candidate.record,
                        &self.records,
                        &self.aliases,
                        &self.ambiguous_aliases,
                    )
                })
                .map(|(canonical, _)| canonical.clone())
                .collect::<Vec<_>>();
            if eligible.is_empty() {
                break;
            }
            for canonical in eligible {
                let Some(candidate) = self.candidates.remove(&canonical) else {
                    continue;
                };
                self.conditional = true;
                self.accounting.retained_candidates =
                    self.accounting.retained_candidates.saturating_add(1);
                self.conditional_consequences.insert(
                    canonical.clone(),
                    (
                        candidate.sources.clone(),
                        StopImpactConditionalClassification::ConditionallyAffected,
                    ),
                );
                for source in &candidate.sources {
                    if self.edges.len() >= MAX_EDGES {
                        self.accounting.omitted_known_edges =
                            self.accounting.omitted_known_edges.saturating_add(1);
                        self.incomplete = true;
                        self.warning(StopImpactWarning::EdgeLimit);
                        continue;
                    }
                    self.edges.insert(
                        (
                            source.clone(),
                            canonical.clone(),
                            relationship_code(StopImpactRelationship::StopWhenUnneededCandidate),
                        ),
                        SystemdStopImpactEdge {
                            source: source.clone(),
                            target: canonical.clone(),
                            relationship: StopImpactRelationship::StopWhenUnneededCandidate,
                            provenance: StopImpactProvenance::Conditional,
                        },
                    );
                }
                self.records.insert(
                    canonical.clone(),
                    Retained {
                        record: candidate.record,
                        depth: candidate.depth,
                        provenance: StopImpactProvenance::Conditional,
                        candidate: true,
                    },
                );
                self.inspect_record(&canonical);
            }
            if self.has_direct_frontier() {
                break;
            }
        }
    }

    fn canonicalize_edges(&mut self, requested: &str, canonical: &str) {
        if requested == canonical {
            return;
        }
        let previous = std::mem::take(&mut self.edges);
        for (_, mut edge) in previous {
            if edge.source == requested {
                edge.source = canonical.to_owned();
            }
            if edge.target == requested {
                edge.target = canonical.to_owned();
            }
            self.edges
                .entry((
                    edge.source.clone(),
                    edge.target.clone(),
                    relationship_code(edge.relationship),
                ))
                .or_insert(edge);
        }
    }

    fn finish(mut self, root_final: Option<ParsedStopImpactUnit>) -> SystemdStopImpactAssessment {
        let root_consistent = root_final
            .as_ref()
            .is_some_and(|final_record| final_record == &self.root_initial);
        if !root_consistent {
            self.incomplete = true;
            self.unknown = true;
            self.warning(StopImpactWarning::ConcurrentTopologyChange);
        }
        if !self.pending.is_empty() {
            if self.pending.iter().any(|pending| pending.candidate) {
                self.unknown = true;
                self.warning(StopImpactWarning::CandidateCoverageIncomplete);
            }
            self.accounting.unresolved_frontier_references = self
                .accounting
                .unresolved_frontier_references
                .saturating_add(u32::try_from(self.pending.len()).unwrap_or(u32::MAX));
            self.incomplete = true;
            self.warning(StopImpactWarning::QueryLimit);
        }
        let remaining_candidates = std::mem::take(&mut self.candidates);
        let traversal_incomplete = self.incomplete;
        for (canonical, candidate) in &remaining_candidates {
            self.accounting.omitted_known_candidates =
                self.accounting.omitted_known_candidates.saturating_add(1);
            let mut unresolved_reverse = BTreeSet::new();
            let mut observed_unaffected_reverse = false;
            for name in CANDIDATE_REVERSE
                .iter()
                .flat_map(|property| candidate.record.related(property))
            {
                if self.ambiguous_aliases.contains(name) {
                    unresolved_reverse.insert(name.clone());
                    continue;
                }
                let resolved = self.aliases.get(name).unwrap_or(name);
                if self.records.contains_key(resolved) {
                    continue;
                }
                if remaining_candidates.contains_key(resolved) {
                    observed_unaffected_reverse = true;
                } else {
                    unresolved_reverse.insert(name.clone());
                }
            }
            let classification = if !candidate.record.stop_when_unneeded {
                StopImpactConditionalClassification::NotStopWhenUnneeded
            } else if traversal_incomplete
                || !unresolved_reverse.is_empty()
                || candidate.record.has_unsupported_enumerant()
            {
                self.incomplete = true;
                self.unknown = true;
                self.accounting.unresolved_frontier_references = self
                    .accounting
                    .unresolved_frontier_references
                    .saturating_add(u32::try_from(unresolved_reverse.len()).unwrap_or(u32::MAX));
                self.warning(StopImpactWarning::CandidateCoverageIncomplete);
                StopImpactConditionalClassification::CoverageUnknown
            } else if observed_unaffected_reverse {
                self.conditional = true;
                StopImpactConditionalClassification::RetainedByUnaffectedReference
            } else {
                self.incomplete = true;
                self.unknown = true;
                self.warning(StopImpactWarning::CandidateCoverageIncomplete);
                StopImpactConditionalClassification::CoverageUnknown
            };
            if candidate.record.has_unsupported_enumerant() {
                self.incomplete = true;
                self.unknown = true;
                self.warning(StopImpactWarning::UnsupportedEnumerant);
            }
            self.conditional_consequences.insert(
                canonical.clone(),
                (candidate.sources.clone(), classification),
            );
        }
        if self.incomplete {
            self.unknown = true;
        }
        let mut projection = DiagnosticProjection::new();
        for record in self
            .records
            .values()
            .map(|retained| &retained.record)
            .chain(
                remaining_candidates
                    .values()
                    .map(|candidate| &candidate.record),
            )
        {
            projection.project_record(record, &self.aliases, &self.ambiguous_aliases);
        }
        let omitted_diagnostics = projection.unique_count.saturating_sub(MAX_DIAGNOSTICS);
        let projected_diagnostics = projection.retained;
        if omitted_diagnostics > 0 {
            self.accounting.omitted_known_diagnostics = self
                .accounting
                .omitted_known_diagnostics
                .saturating_add(u32::try_from(omitted_diagnostics).unwrap_or(u32::MAX));
            self.incomplete = true;
            self.unknown = true;
            self.warning(StopImpactWarning::DiagnosticLimit);
        }
        let conditional_diagnostics = projected_diagnostics;
        let mut units = self
            .records
            .into_values()
            .map(|retained| SystemdStopImpactUnit {
                canonical_unit: retained.record.id,
                load_state: retained.record.load_state,
                active_state: retained.record.active_state,
                sub_state: retained.record.sub_state,
                can_stop: retained.record.can_stop,
                refuse_manual_stop: retained.record.refuse_manual_stop,
                stop_when_unneeded: retained.record.stop_when_unneeded,
                has_pending_job: retained.record.has_pending_job,
                depth: retained.depth,
                provenance: retained.provenance,
            })
            .collect::<Vec<_>>();
        units.sort_by(|left, right| left.canonical_unit.cmp(&right.canonical_unit));
        let edges = self.edges.into_values().collect::<Vec<_>>();
        let conditional_consequences = self
            .conditional_consequences
            .into_iter()
            .map(|(canonical_unit, (sources, classification))| {
                SystemdStopImpactConditionalConsequence {
                    canonical_unit,
                    sources: sources.into_iter().collect(),
                    classification,
                }
            })
            .collect::<Vec<_>>();
        self.accounting.retained_units = u32::try_from(units.len()).unwrap_or(u32::MAX);
        self.accounting.retained_edges = u32::try_from(edges.len()).unwrap_or(u32::MAX);
        self.accounting.retained_diagnostics =
            u32::try_from(conditional_diagnostics.len()).unwrap_or(u32::MAX);
        let warnings = self
            .warnings
            .into_iter()
            .filter_map(warning_from_code)
            .collect::<Vec<_>>();
        SystemdStopImpactAssessment {
            host_id: self.host_id,
            host_session_id: self.host_session_id,
            root_unit: self.root_initial.id,
            observed_at: chrono::Utc::now().to_rfc3339(),
            completeness: if self.incomplete {
                StopImpactCompleteness::Partial
            } else {
                StopImpactCompleteness::Complete
            },
            uncertainty: if self.unknown {
                StopImpactUncertainty::UnknownImpact
            } else if self.conditional {
                StopImpactUncertainty::ConditionalImpact
            } else {
                StopImpactUncertainty::DirectOnly
            },
            root_consistent,
            units,
            edges,
            conditional_consequences,
            conditional_diagnostics,
            accounting: self.accounting,
            warnings,
            limitations: vec![
                "This diagnostic covers systemd-managed topology only; application dependencies, stop hooks, process ownership, and SSH, VPN, or network access effects may be unknown.".into(),
                "No dependency relationship is evidence that stopping a unit is safe.".into(),
            ],
        }
    }
}

fn match_batch_by_identity(
    requested: &[Pending],
    parsed: Vec<ParsedStopImpactUnit>,
) -> Result<HashMap<String, ParsedStopImpactUnit>, AppError> {
    if requested.len() != parsed.len() {
        return Err(invalid());
    }
    let mut matched = HashMap::new();
    let mut canonical_ids = HashSet::new();
    for record in parsed {
        if !canonical_ids.insert(record.id.clone()) {
            return Err(invalid());
        }
        let identities = requested
            .iter()
            .enumerate()
            .filter(|(_, pending)| {
                record.id == pending.requested || record.names.contains(&pending.requested)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if identities.len() != 1 {
            return Err(invalid());
        }
        let requested_identity = requested[identities[0]].requested.clone();
        if matched.insert(requested_identity, record).is_some() {
            return Err(invalid());
        }
    }
    if requested
        .iter()
        .any(|pending| !matched.contains_key(&pending.requested))
    {
        return Err(invalid());
    }
    Ok(matched)
}

fn candidate_affected(
    record: &ParsedStopImpactUnit,
    affected: &HashMap<String, Retained>,
    aliases: &HashMap<String, String>,
    ambiguous_aliases: &HashSet<String>,
) -> bool {
    if !record.stop_when_unneeded {
        return false;
    }
    CANDIDATE_REVERSE.iter().all(|property| {
        record.related(property).iter().all(|name| {
            if ambiguous_aliases.contains(name) {
                return false;
            }
            aliases.get(name).map_or_else(
                || affected.contains_key(name),
                |canonical| affected.contains_key(canonical),
            )
        })
    })
}

pub async fn assess_systemd_stop_impact(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
    root_unit: String,
) -> Result<SystemdStopImpactAssessment, AppError> {
    let engine = SystemdStopImpactEngine;
    let deadline = Instant::now() + TOTAL_TIMEOUT;
    let root_query = SystemdStopImpactQuery::single(root_unit.clone())?;
    let initial_output = engine
        .execute(session, &root_query, cancellation.clone())
        .await?;
    let mut initial = parse_systemd_stop_impact(&initial_output)?;
    if initial.len() != 1 {
        return Err(invalid());
    }
    let mut builder = Builder::new(
        host_id,
        host_session_id,
        root_unit,
        initial.remove(0),
        initial_output.len(),
    );
    let root = builder.root_initial.id.clone();
    builder.inspect_record(&root);
    if !builder.enforce_working_set() {
        return Err(invalid());
    }

    // Q2-Q5 are the only traversal queries. One real query is reserved for the
    // final root consistency check throughout traversal; no dummy query occurs.
    while builder.accounting.actual_ssh_queries < MAX_QUERIES - 1 && !builder.pending.is_empty() {
        let traversal_deadline = deadline
            .checked_sub(FINAL_QUERY_RESERVE)
            .unwrap_or(deadline);
        if Instant::now() >= traversal_deadline {
            builder.incomplete = true;
            builder.warning(StopImpactWarning::TimeLimit);
            break;
        }
        let batch = builder.take_batch();
        let query =
            SystemdStopImpactQuery::new(batch.iter().map(|pending| pending.requested.clone()))?;
        let response = timeout_at(
            traversal_deadline,
            engine.execute(session, &query, cancellation.clone()),
        )
        .await;
        builder.accounting.actual_ssh_queries += 1;
        match response {
            Ok(Ok(output)) => {
                if builder.aggregate_output_bytes.saturating_add(output.len())
                    > MAX_AGGREGATE_OUTPUT_BYTES
                {
                    builder.incomplete = true;
                    builder.unknown = true;
                    builder.accounting.unresolved_frontier_references = builder
                        .accounting
                        .unresolved_frontier_references
                        .saturating_add(u32::try_from(batch.len()).unwrap_or(u32::MAX));
                    if batch.iter().any(|pending| pending.candidate) {
                        builder.warning(StopImpactWarning::CandidateCoverageIncomplete);
                    }
                    builder.warning(StopImpactWarning::OutputLimit);
                    break;
                }
                builder.aggregate_output_bytes += output.len();
                let parsed = parse_systemd_stop_impact(&output)
                    .and_then(|parsed| builder.retain_batch(batch.clone(), parsed));
                if parsed.is_err() {
                    builder.incomplete = true;
                    builder.unknown = true;
                    builder.accounting.unresolved_frontier_references = builder
                        .accounting
                        .unresolved_frontier_references
                        .saturating_add(u32::try_from(batch.len()).unwrap_or(u32::MAX));
                    if batch.iter().any(|pending| pending.candidate) {
                        builder.warning(StopImpactWarning::CandidateCoverageIncomplete);
                    }
                    builder.warning(StopImpactWarning::UnsupportedProperty);
                    break;
                }
                if !builder.enforce_working_set() {
                    break;
                }
            }
            Ok(Err(error)) if error.code == ErrorCode::Cancelled => return Err(error),
            Ok(Err(_)) | Err(_) => {
                builder.incomplete = true;
                builder.unknown = true;
                if batch.iter().any(|pending| pending.candidate) {
                    builder.warning(StopImpactWarning::CandidateCoverageIncomplete);
                }
                builder.accounting.unresolved_frontier_references = builder
                    .accounting
                    .unresolved_frontier_references
                    .saturating_add(u32::try_from(batch.len()).unwrap_or(u32::MAX));
                builder.warning(StopImpactWarning::QueryLimit);
                break;
            }
        }
    }

    if cancellation.is_cancelled() {
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The operation was cancelled.",
        ));
    }
    // This is always the final actual query. Root is the sole approved duplicate.
    builder.accounting.actual_ssh_queries += 1;
    let final_root = match timeout_at(
        deadline,
        engine.execute(session, &root_query, cancellation.clone()),
    )
    .await
    {
        Ok(Ok(output)) => {
            if builder.aggregate_output_bytes.saturating_add(output.len())
                > MAX_AGGREGATE_OUTPUT_BYTES
            {
                builder.warning(StopImpactWarning::OutputLimit);
                builder.incomplete = true;
                builder.unknown = true;
                return Ok(builder.finish(None));
            }
            builder.aggregate_output_bytes += output.len();
            match parse_systemd_stop_impact(&output) {
                Ok(mut parsed) if parsed.len() == 1 => Some(parsed.remove(0)),
                Ok(_) | Err(_) => {
                    builder.warning(StopImpactWarning::UnsupportedProperty);
                    builder.incomplete = true;
                    builder.unknown = true;
                    None
                }
            }
        }
        Ok(Err(error)) if error.code == ErrorCode::Cancelled => return Err(error),
        Ok(Err(_)) | Err(_) => None,
    };
    Ok(builder.finish(final_root))
}

fn relationship_code(value: StopImpactRelationship) -> u8 {
    match value {
        StopImpactRelationship::RequiredBy => 0,
        StopImpactRelationship::BoundBy => 1,
        StopImpactRelationship::ConsistsOf => 2,
        StopImpactRelationship::PropagatesStopTo => 3,
        StopImpactRelationship::StopWhenUnneededCandidate => 4,
    }
}

fn diagnostic_code(value: StopImpactDiagnosticKind) -> u8 {
    match value {
        StopImpactDiagnosticKind::OnSuccessActivation => 0,
        StopImpactDiagnosticKind::OnFailureActivation => 1,
        StopImpactDiagnosticKind::Trigger => 2,
        StopImpactDiagnosticKind::TriggeredBy => 3,
        StopImpactDiagnosticKind::UpheldByReactivation => 9,
        StopImpactDiagnosticKind::NonDefaultOnSuccessJobMode => 4,
        StopImpactDiagnosticKind::NonDefaultOnFailureJobMode => 5,
        StopImpactDiagnosticKind::SuccessManagerAction => 6,
        StopImpactDiagnosticKind::FailureManagerAction => 7,
        StopImpactDiagnosticKind::UnsupportedEnumerant => 8,
    }
}

fn warning_code(value: StopImpactWarning) -> u8 {
    match value {
        StopImpactWarning::NodeLimit => 0,
        StopImpactWarning::EdgeLimit => 1,
        StopImpactWarning::DepthLimit => 2,
        StopImpactWarning::QueryLimit => 3,
        StopImpactWarning::TimeLimit => 4,
        StopImpactWarning::OutputLimit => 5,
        StopImpactWarning::AliasAmbiguity => 6,
        StopImpactWarning::UnsupportedProperty => 7,
        StopImpactWarning::UnsupportedEnumerant => 8,
        StopImpactWarning::ConcurrentTopologyChange => 9,
        StopImpactWarning::CandidateCoverageIncomplete => 10,
        StopImpactWarning::DiagnosticLimit => 11,
    }
}

fn warning_from_code(value: u8) -> Option<StopImpactWarning> {
    Some(match value {
        0 => StopImpactWarning::NodeLimit,
        1 => StopImpactWarning::EdgeLimit,
        2 => StopImpactWarning::DepthLimit,
        3 => StopImpactWarning::QueryLimit,
        4 => StopImpactWarning::TimeLimit,
        5 => StopImpactWarning::OutputLimit,
        6 => StopImpactWarning::AliasAmbiguity,
        7 => StopImpactWarning::UnsupportedProperty,
        8 => StopImpactWarning::UnsupportedEnumerant,
        9 => StopImpactWarning::ConcurrentTopologyChange,
        10 => StopImpactWarning::CandidateCoverageIncomplete,
        11 => StopImpactWarning::DiagnosticLimit,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    fn record(id: &str, direct: &str, requires: &str, swu: bool) -> String {
        [
            format!("Id={id}"),
            format!("Names={id}"),
            "Following=".into(),
            "LoadState=loaded".into(),
            "ActiveState=active".into(),
            "SubState=running".into(),
            "CanStop=yes".into(),
            "RefuseManualStop=no".into(),
            "Job=".into(),
            "NeedDaemonReload=no".into(),
            format!("StopWhenUnneeded={}", if swu { "yes" } else { "no" }),
            format!("Requires={requires}"),
            format!("RequiredBy={direct}"),
            "Requisite=".into(),
            "RequisiteOf=".into(),
            "Wants=".into(),
            "WantedBy=".into(),
            "BindsTo=".into(),
            "BoundBy=".into(),
            "PartOf=".into(),
            "ConsistsOf=".into(),
            "PropagatesStopTo=".into(),
            "StopPropagatedFrom=".into(),
            "Upholds=".into(),
            "UpheldBy=".into(),
            "Conflicts=".into(),
            "ConflictedBy=".into(),
            "Before=".into(),
            "After=".into(),
            "Triggers=".into(),
            "TriggeredBy=".into(),
            "OnSuccess=".into(),
            "OnFailure=".into(),
            "OnSuccessJobMode=fail".into(),
            "OnFailureJobMode=replace".into(),
            "SuccessAction=none".into(),
            "FailureAction=none".into(),
        ]
        .join("\n")
    }

    #[test]
    fn strict_parser_accepts_exact_contract_and_rejects_duplicates_missing_and_paths() {
        let parsed = parse_systemd_stop_impact(&record(
            "root.service",
            "child.service",
            "helper.service",
            false,
        ))
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "root.service");
        assert_eq!(parsed[0].related("RequiredBy"), &["child.service"]);
        assert!(!parsed[0].has_pending_job);

        let duplicate = format!(
            "{}\nId=again.service",
            record("root.service", "", "", false)
        );
        assert!(parse_systemd_stop_impact(&duplicate).is_err());
        let missing = record("root.service", "", "", false).replace("CanStop=yes\n", "");
        assert!(parse_systemd_stop_impact(&missing).is_err());
        let raw_job_path = record("root.service", "", "", false)
            .replace("Job=", "Job=/org/freedesktop/systemd1/job/42");
        assert!(parse_systemd_stop_impact(&raw_job_path).is_err());
        for invalid_job in ["0", "4294967296", "+1", "-1", "1.0"] {
            let invalid = record("root.service", "", "", false)
                .replace("Job=", &format!("Job={invalid_job}"));
            assert!(parse_systemd_stop_impact(&invalid).is_err());
        }
        let valid_job = record("root.service", "", "", false).replace("Job=", "Job=4294967295");
        assert!(parse_systemd_stop_impact(&valid_job).unwrap()[0].has_pending_job);

        let quoted = record("canonical.service", "", "", false)
            .replace(
                "Names=canonical.service",
                "Names='alias.service' canonical.service",
            )
            .replace("Following=", "Following=canonical.service");
        let parsed = parse_systemd_stop_impact(&quoted).unwrap();
        assert_eq!(parsed[0].id, "canonical.service");
        assert_eq!(parsed[0].names, ["alias.service", "canonical.service"]);

        let unterminated = record("root.service", "", "", false)
            .replace("Names=root.service", "Names='root.service");
        assert!(parse_systemd_stop_impact(&unterminated).is_err());
    }

    fn pending(requested: &str) -> Pending {
        Pending {
            requested: requested.into(),
            depth: 1,
            provenance: StopImpactProvenance::Direct,
            candidate: false,
            candidate_sources: BTreeSet::new(),
            relationship_order: 0,
        }
    }

    #[test]
    fn systemd_v255_cli_relationship_lists_preserve_canonical_hex_escapes() {
        // systemd v255, db11bab38ccf1ed257f310d29070843d4c58ea01:
        // src/shared/bus-print-properties.c: string arrays use shell_maybe_quote(str, 0).
        // src/basic/escape.c: shell_maybe_quote/strcpy_backslash_escaped and
        // src/basic/escape.h: SHELL_NEED_ESCAPE make each literal backslash
        // a doubled backslash inside double quotes. These are synthetic names.
        for (cli, canonical) in [
            ("normal.service", "normal.service"),
            (
                "instance@one-two_3:four.service",
                "instance@one-two_3:four.service",
            ),
            (
                r#""mnt-fixture\\x20one.mount""#,
                r"mnt-fixture\x20one.mount",
            ),
            (r#""escaped\\x2dunit.service""#, r"escaped\x2dunit.service"),
            (
                r#""escaped\\x5C\\x22\\x3b.service""#,
                r"escaped\x5C\x22\x3b.service",
            ),
            (r"escaped\x20unit.service", r"escaped\x20unit.service"),
            (r"'escaped\x20unit.service'", r"escaped\x20unit.service"),
        ] {
            assert_eq!(unit_list(cli).unwrap(), [canonical], "CLI form: {cli}");
        }
        // Canonical hex spelling is opaque identity text, never a decoded byte
        // (including NUL, whitespace, shell syntax, non-ASCII or backslash).
        for byte in 0..=255 {
            for hex in [format!("{byte:02x}"), format!("{byte:02X}")] {
                let canonical = format!(r"escaped\x{hex}.service");
                let cli = format!(r#""escaped\\x{hex}.service""#);
                assert_eq!(unit_list(&cli).unwrap(), [canonical]);
            }
        }
    }

    #[test]
    fn systemd_v255_cli_mixed_batch_and_alias_identity_matching() {
        let list = r#"normal.service "escaped\\x20one.service" "escaped\\x2Dtwo.mount""#;
        let normal = record("normal.service", "", "", false);
        let canonical = r"canonical\x20unit.service";
        let alias = r"alias\x20unit.service";
        let escaped = record(canonical, list, "", false)
            .replace(
                &format!("Names={canonical}"),
                r#"Names="canonical\\x20unit.service" "alias\\x20unit.service""#,
            )
            .replace("Following=", &format!("Following={canonical}"))
            .replace("Before=", &format!("Before={list}"));
        let batch = format!("{normal}\n\n{escaped}\n");
        let parsed = parse_systemd_stop_impact(&batch).unwrap();
        assert_eq!(parsed.len(), 2);
        for property in ["RequiredBy", "Before"] {
            assert_eq!(
                parsed[1].related(property),
                [
                    "normal.service",
                    r"escaped\x20one.service",
                    r"escaped\x2Dtwo.mount"
                ]
            );
        }
        assert_eq!(parsed[1].names, [canonical, alias]);
        assert_eq!(parsed[1].following.as_deref(), Some(canonical));
        let requested = vec![pending("normal.service"), pending(alias)];
        let matched = match_batch_by_identity(&requested, parsed.clone()).unwrap();
        assert_eq!(matched[alias].id, canonical);
        let reversed =
            match_batch_by_identity(&requested, parsed.into_iter().rev().collect()).unwrap();
        assert_eq!(matched[alias], reversed[alias]);
        assert_eq!(matched["normal.service"], reversed["normal.service"]);

        // Two requested aliases still cannot claim the same observed record.
        let ambiguous = vec![pending(canonical), pending(alias)];
        let parsed = parse_systemd_stop_impact(&escaped).unwrap();
        assert!(match_batch_by_identity(&ambiguous, parsed).is_err());
    }

    #[test]
    fn systemd_v255_cli_rejects_malformed_quotes_and_unsupported_escapes() {
        for value in [
            "'unterminated.service",
            "\"unterminated.service",
            "\"\"",
            "''",
            "pre\"mixed\".service",
            "'one'\"two\".service",
            "\"unit.service\"suffix",
            r#""escaped\x20.service""#,
            r#""escaped\q.service""#,
            r#""escaped\n.service""#,
            r#""escaped\t.service""#,
            r#""escaped\040.service""#,
            r#""escaped\u0020.service""#,
            r#""escaped\\\\x20.service""#,
            r"escaped\\x20.service",
            r"escaped\q.service",
            r"escaped\x.service",
            r"escaped\x2.service",
            r"escaped\xGG.service",
            r"escaped\X20.service",
            r#""escaped\\x.service""#,
            r#""escaped\\x2.service""#,
            r#""escaped\\x2z.service""#,
            "trailing.service\\",
            "\"trailing.service\\",
            r"$'escaped\x20.service'",
        ] {
            assert!(
                unit_list(value).is_err(),
                "accepted malformed CLI form: {value}"
            );
        }
    }

    #[test]
    fn systemd_v255_cli_rejects_unsafe_decoded_values_and_normalized_duplicates() {
        for value in [
            "\"white space.service\"",
            "'white space.service'",
            "\"unit;other.service\"",
            r#""unit\$value.service""#,
            r#""unit\`value.service""#,
            r#""unit\"value.service""#,
            "unit&other.service",
            "unit|other.service",
            "unit(1).service",
            "unit*.service",
            "unit>other.service",
            "unit<other.service",
            "unit!other.service",
            "unit/other.service",
            "unit=value.service",
            "unité.service",
            r#"escaped\x20unit.service "escaped\\x20unit.service""#,
            r#"'escaped\x20unit.service' "escaped\\x20unit.service""#,
            r#""escaped\\x20unit.service" "escaped\\x20unit.service""#,
            "normal.service 'normal.service'",
        ] {
            assert!(
                unit_list(value).is_err(),
                "accepted unsafe/duplicate identities: {value}"
            );
        }
        for byte in (0..=31).chain(std::iter::once(127)) {
            for value in [
                format!("unit{}name.service", char::from(byte)),
                format!("normal.service{}other.service", char::from(byte)),
            ] {
                assert!(unit_list(&value).is_err(), "accepted control byte: {byte}");
            }
        }
    }

    #[test]
    fn systemd_v255_cli_rejects_entire_malformed_mixed_batches() {
        let normal = record("normal.service", "", "", false);
        for property in ["Names", "RequiredBy", "Before"] {
            for cli in [
                r#""escaped\\x2z.service""#,
                r#""escaped\\x20.service" escaped\x20.service"#,
                "\"unterminated.service",
            ] {
                let original = if property == "Names" {
                    "Names=other.service".to_owned()
                } else {
                    format!("{property}=")
                };
                let malformed = record("other.service", "", "", false)
                    .replace(&original, &format!("{property}={cli}"));
                for batch in [
                    format!("{normal}\n\n{malformed}"),
                    format!("{malformed}\n\n{normal}"),
                ] {
                    assert!(parse_systemd_stop_impact(&batch).is_err());
                }
            }
        }
    }

    #[test]
    fn systemd_v255_cli_normalization_keeps_encoded_property_and_unit_bounds() {
        let prefix = "a".repeat(243);
        let canonical = format!(r"{prefix}\x20.service");
        let cli = format!(r#""{prefix}\\x20.service""#);
        assert_eq!(canonical.len(), 255);
        assert!(cli.len() > canonical.len());
        assert_eq!(unit_list(&cli).unwrap(), [canonical]);
        let oversized = format!(r#""a{prefix}\\x20.service""#);
        assert!(unit_list(&oversized).is_err());

        // The original encoded property is bounded before normalization, even
        // when decoding/tokenization would produce only two short identities.
        let oversized_property = format!("{}{}other.service", cli, " ".repeat(MAX_PROPERTY_BYTES));
        let batch = record("root.service", &oversized_property, "", false);
        assert!(parse_systemd_stop_impact(&batch).is_err());
    }

    #[test]
    fn identity_matching_rejects_missing_extra_duplicate_and_ambiguous_records() {
        let requested = vec![pending("a.service"), pending("b.service")];
        let a = parse_systemd_stop_impact(&record("a.service", "", "", false))
            .unwrap()
            .remove(0);
        let b = parse_systemd_stop_impact(&record("b.service", "", "", false))
            .unwrap()
            .remove(0);
        let c = parse_systemd_stop_impact(&record("c.service", "", "", false))
            .unwrap()
            .remove(0);
        assert!(match_batch_by_identity(&requested, vec![a.clone()]).is_err());
        assert!(match_batch_by_identity(&requested, vec![a.clone(), b.clone(), c]).is_err());
        assert!(match_batch_by_identity(&requested, vec![a.clone(), a]).is_err());

        let aliases = record("canonical.service", "", "", false).replace(
            "Names=canonical.service",
            "Names=a.service b.service canonical.service",
        );
        let ambiguous = parse_systemd_stop_impact(&aliases).unwrap().remove(0);
        assert!(match_batch_by_identity(&requested, vec![ambiguous, b]).is_err());
    }

    #[test]
    fn pending_frontier_upgrades_and_orders_by_depth_name_and_relationship() {
        let root = parse_systemd_stop_impact(&record("root.service", "", "", false))
            .unwrap()
            .remove(0);
        let mut builder = Builder::new(
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
            root,
            0,
        );
        builder.admit_query(
            "shared.service",
            2,
            StopImpactProvenance::Conditional,
            true,
            Some("root.service"),
            9,
        );
        builder.admit_query(
            "shared.service",
            1,
            StopImpactProvenance::Direct,
            false,
            None,
            3,
        );
        builder.admit_query(
            "shared.service",
            1,
            StopImpactProvenance::Direct,
            false,
            None,
            1,
        );
        builder.admit_query("z.service", 1, StopImpactProvenance::Direct, false, None, 0);
        builder.admit_query("a.service", 2, StopImpactProvenance::Direct, false, None, 0);
        let batch = builder.take_batch();
        assert_eq!(
            batch
                .iter()
                .map(|pending| pending.requested.as_str())
                .collect::<Vec<_>>(),
            ["shared.service", "z.service", "a.service"]
        );
        assert_eq!(batch[0].relationship_order, 1);
        assert_eq!(batch[0].provenance, StopImpactProvenance::Direct);
        assert!(!batch[0].candidate);
    }

    struct Session {
        responses: Mutex<VecDeque<String>>,
        commands: Arc<Mutex<Vec<String>>>,
    }

    #[tokio::test]
    async fn systemd_v255_cli_candidate_batch_keeps_valid_records_and_final_root_query() {
        let helper_id = "helper.service";
        let helper_alias = "helper-alias.service";
        let helper = record(helper_id, "root.service", "", true).replace(
            &format!("Names={helper_id}"),
            "Names=helper.service helper-alias.service",
        );
        let slice = record(
            "system.slice",
            r#"root.service helper.service "mnt-fixture\\x20one.mount""#,
            "",
            false,
        )
        .replace(
            "Before=",
            r#"Before=child.service "mnt-fixture\\x20one.mount""#,
        );
        let child = record("child.service", "", "", false);
        let mut results = Vec::new();
        let mut request_sets = Vec::new();
        for reversed in [false, true] {
            let requires = if reversed {
                "system.slice helper-alias.service"
            } else {
                "helper-alias.service system.slice"
            };
            let root = record("root.service", "child.service", requires, false)
                .replace("OnSuccess=", "OnSuccess=notify.service");
            let batch = if reversed {
                [slice.as_str(), helper.as_str()]
            } else {
                [helper.as_str(), slice.as_str()]
            }
            .join("\n\n");
            let commands = Arc::new(Mutex::new(Vec::new()));
            let session = Session {
                responses: Mutex::new(VecDeque::from([root.clone(), child.clone(), batch, root])),
                commands: commands.clone(),
            };
            let result = assess_systemd_stop_impact(
                &session,
                CancellationToken::new(),
                HostId::new(),
                HostSessionId::new(),
                "root.service".into(),
            )
            .await
            .unwrap();
            assert_eq!(result.completeness, StopImpactCompleteness::Complete);
            assert!(result.root_consistent);
            assert_eq!(result.accounting.actual_ssh_queries, 4);
            assert_eq!(result.accounting.retained_units, 3);
            assert_eq!(result.accounting.unresolved_frontier_references, 0);
            assert!(result.conditional_consequences.iter().any(|consequence| {
                consequence.canonical_unit == helper_id
                    && consequence.classification
                        == StopImpactConditionalClassification::ConditionallyAffected
            }));
            assert!(
                !result
                    .warnings
                    .contains(&StopImpactWarning::UnsupportedProperty)
            );
            let requests = commands.lock().unwrap().clone();
            assert_eq!(requests.len(), 4);
            assert!(requests[2].ends_with(&format!(" show -- '{helper_alias}' 'system.slice'")));
            assert!(
                requests
                    .last()
                    .unwrap()
                    .ends_with(" show -- 'root.service'")
            );
            request_sets.push(requests);
            results.push(result);
        }
        assert_eq!(request_sets[0], request_sets[1]);
        assert_eq!(results[0].units, results[1].units);
        assert_eq!(results[0].edges, results[1].edges);
        assert_eq!(
            results[0].conditional_consequences,
            results[1].conditional_consequences
        );
        assert_eq!(
            results[0].conditional_diagnostics,
            results[1].conditional_diagnostics
        );
        assert_eq!(results[0].accounting, results[1].accounting);
    }

    struct FallibleSession {
        responses: Mutex<VecDeque<Result<String, AppError>>>,
        commands: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl RemoteSession for Session {
        async fn execute(
            &self,
            _: nexus_operations::ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            unreachable!()
        }

        async fn execute_stop_impact(
            &self,
            query: &SystemdStopImpactQuery,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            self.commands.lock().unwrap().push(query.command().into());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(invalid)
        }

        async fn disconnect(&self) -> Result<(), AppError> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    #[async_trait]
    impl RemoteSession for FallibleSession {
        async fn execute(
            &self,
            _: nexus_operations::ReadOnlyCommand,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            unreachable!()
        }

        async fn execute_stop_impact(
            &self,
            query: &SystemdStopImpactQuery,
            _: CancellationToken,
        ) -> Result<String, AppError> {
            self.commands.lock().unwrap().push(query.command().into());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(invalid)?
        }

        async fn disconnect(&self) -> Result<(), AppError> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn schedules_nonempty_batches_and_final_root_query_last() {
        let root = record("root.service", "child.service", "helper.service", false);
        let child = record("child.service", "", "", false);
        let helper = record("helper.service", "", "", true);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), child, helper, root])),
            commands: commands.clone(),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert!(result.root_consistent);
        assert_eq!(result.accounting.actual_ssh_queries, 4);
        assert_eq!(commands.lock().unwrap().len(), 4);
        assert!(commands.lock().unwrap()[1].ends_with(" show -- 'child.service'"));
        assert!(commands.lock().unwrap()[2].ends_with(" show -- 'helper.service'"));
        assert!(commands.lock().unwrap()[3].ends_with(" show -- 'root.service'"));
    }

    #[tokio::test]
    async fn matches_reversed_multi_unit_response_by_validated_identity() {
        let root = record("root.service", "b.service a.service", "", false);
        let a = record("a.service", "", "", false);
        let b = record("b.service", "", "", false);
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), [a, b].join("\n\n"), root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Complete);
        assert_eq!(result.accounting.retained_units, 3);
    }

    #[tokio::test]
    async fn upgrades_cached_conditional_provenance_without_duplicate_query() {
        let root = record("root.service", "child.service", "helper.service", false);
        let child = record("child.service", "helper.service", "", false);
        let helper = record("helper.service", "root.service child.service", "", true);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), child, helper, root])),
            commands: commands.clone(),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        let helper = result
            .units
            .iter()
            .find(|unit| unit.canonical_unit == "helper.service")
            .unwrap();
        assert_eq!(helper.provenance, StopImpactProvenance::Direct);
        assert_eq!(result.accounting.actual_ssh_queries, 4);
        let commands = commands.lock().unwrap();
        assert_eq!(commands.len(), 4);
        assert!(commands[1].ends_with(" show -- 'child.service'"));
        assert!(commands[2].ends_with(" show -- 'helper.service'"));
        assert!(commands[3].ends_with(" show -- 'root.service'"));
    }

    #[tokio::test]
    async fn omits_ineligible_conditional_candidate_without_claiming_unknown_nodes() {
        let root = record("root.service", "", "helper.service", false);
        let helper = record("helper.service", "", "", false);
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), helper, root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Complete);
        assert_eq!(result.accounting.observed_candidate_references, 1);
        assert_eq!(result.accounting.omitted_known_candidates, 1);
        assert_eq!(result.accounting.retained_candidates, 0);
        assert_eq!(result.accounting.retained_units, 1);
        assert_eq!(result.conditional_consequences.len(), 1);
        assert_eq!(
            result.conditional_consequences[0].classification,
            StopImpactConditionalClassification::NotStopWhenUnneeded
        );
    }

    #[tokio::test]
    async fn classifies_unknown_reverse_reference_as_incomplete_coverage() {
        let root = record("root.service", "", "helper.service", false);
        let helper = record("helper.service", "external.service", "", true);
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), helper, root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert_eq!(result.accounting.unresolved_frontier_references, 1);
        assert_eq!(
            result.conditional_consequences[0].classification,
            StopImpactConditionalClassification::CoverageUnknown
        );
    }

    #[tokio::test]
    async fn classifies_observed_unaffected_reverse_reference_explicitly() {
        let root = record("root.service", "", "helper.service external.service", false);
        let helper = record("helper.service", "external.service", "", true);
        let external = record("external.service", "", "", false);
        let session = Session {
            responses: Mutex::new(VecDeque::from([
                root.clone(),
                [helper, external].join("\n\n"),
                root,
            ])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Complete);
        assert_eq!(result.uncertainty, StopImpactUncertainty::ConditionalImpact);
        let helper = result
            .conditional_consequences
            .iter()
            .find(|consequence| consequence.canonical_unit == "helper.service")
            .unwrap();
        assert_eq!(
            helper.classification,
            StopImpactConditionalClassification::RetainedByUnaffectedReference
        );
    }

    #[tokio::test]
    async fn malformed_non_root_batch_stops_traversal_and_final_root_is_still_revalidated() {
        let root = record("root.service", "child.service later.service", "", false);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            responses: Mutex::new(VecDeque::from([
                root.clone(),
                "Id=malformed.service".into(),
                root,
            ])),
            commands: commands.clone(),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert!(result.root_consistent);
        assert_eq!(commands.lock().unwrap().len(), 3);
        assert!(commands.lock().unwrap()[2].ends_with(" show -- 'root.service'"));
    }

    #[tokio::test]
    async fn failed_non_root_batch_stops_traversal_and_final_root_is_still_revalidated() {
        let root = record("root.service", "child.service later.service", "", false);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = FallibleSession {
            responses: Mutex::new(VecDeque::from([
                Ok(root.clone()),
                Err(AppError::new(ErrorCode::Discovery, "query failed")),
                Ok(root),
            ])),
            commands: commands.clone(),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert!(result.root_consistent);
        let commands = commands.lock().unwrap();
        assert_eq!(commands.len(), 3);
        assert!(commands[2].ends_with(" show -- 'root.service'"));
    }

    #[tokio::test]
    async fn unsupported_modes_and_actions_are_partial_with_explicit_diagnostic_evidence() {
        let root = record("root.service", "", "", false)
            .replace("OnSuccessJobMode=fail", "OnSuccessJobMode=future-mode");
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert!(
            result
                .warnings
                .contains(&StopImpactWarning::UnsupportedEnumerant)
        );
        assert_eq!(
            result.conditional_diagnostics,
            [SystemdStopImpactDiagnostic {
                canonical_unit: "root.service".into(),
                kind: StopImpactDiagnosticKind::UnsupportedEnumerant,
                related_unit: None,
                job_mode: None,
                manager_action: None,
            }]
        );
    }

    #[tokio::test]
    async fn publishes_typed_passive_conditional_context_without_raw_enumerants() {
        let root = record("root.service", "", "", false)
            .replace("OnSuccess=", "OnSuccess=next.service")
            .replace("OnFailure=", "OnFailure=recover.service")
            .replace("Triggers=", "Triggers=timer.timer")
            .replace("TriggeredBy=", "TriggeredBy=watch.path")
            .replace(
                "OnFailureJobMode=replace",
                "OnFailureJobMode=restart-dependencies",
            )
            .replace("SuccessAction=none", "SuccessAction=reboot");
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Complete);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert_eq!(result.conditional_diagnostics.len(), 6);
        assert!(result.conditional_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == StopImpactDiagnosticKind::OnSuccessActivation
                && diagnostic.related_unit.as_deref() == Some("next.service")
        }));
        assert!(result.conditional_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == StopImpactDiagnosticKind::TriggeredBy
                && diagnostic.related_unit.as_deref() == Some("watch.path")
        }));
        assert!(result.conditional_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == StopImpactDiagnosticKind::SuccessManagerAction
                && diagnostic.manager_action == Some(StopImpactManagerAction::Reboot)
        }));
        assert!(result.conditional_diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == StopImpactDiagnosticKind::NonDefaultOnFailureJobMode
                && diagnostic.job_mode == Some(StopImpactJobMode::RestartDependencies)
        }));
        assert!(result.conditional_diagnostics.iter().all(|diagnostic| {
            diagnostic.related_unit.as_deref() != Some("reboot")
                && diagnostic.related_unit.as_deref() != Some("restart-dependencies")
        }));
    }

    #[tokio::test]
    async fn publishes_upheld_by_as_passive_unknown_reactivation_context() {
        let root =
            record("root.service", "", "", false).replace("UpheldBy=", "UpheldBy=guardian.service");
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Complete);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert!(result.edges.is_empty());
        assert_eq!(result.accounting.retained_edges, 0);
        assert_eq!(result.accounting.omitted_known_edges, 0);
        assert_eq!(result.accounting.retained_diagnostics, 1);
        assert_eq!(result.accounting.omitted_known_diagnostics, 0);
        assert_eq!(
            result.conditional_diagnostics,
            [SystemdStopImpactDiagnostic {
                canonical_unit: "root.service".into(),
                kind: StopImpactDiagnosticKind::UpheldByReactivation,
                related_unit: Some("guardian.service".into()),
                job_mode: None,
                manager_action: None,
            }]
        );
    }

    #[tokio::test]
    async fn diagnostic_limit_does_not_change_graph_edge_accounting() {
        let references = (0..513)
            .map(|index| format!("u{index:03}.service"))
            .collect::<Vec<_>>()
            .join(" ");
        let root = record("root.service", "", "", false)
            .replace("UpheldBy=", &format!("UpheldBy={references}"));
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert_eq!(result.conditional_diagnostics.len(), MAX_DIAGNOSTICS);
        assert_eq!(result.accounting.retained_diagnostics, 512);
        assert_eq!(result.accounting.omitted_known_diagnostics, 1);
        assert_eq!(result.accounting.retained_edges, 0);
        assert_eq!(result.accounting.omitted_known_edges, 0);
        assert!(
            result
                .warnings
                .contains(&StopImpactWarning::DiagnosticLimit)
        );
        assert!(!result.warnings.contains(&StopImpactWarning::EdgeLimit));
    }

    #[tokio::test]
    async fn graph_edge_and_passive_diagnostic_accounting_are_independent() {
        let references = (0..513)
            .map(|index| format!("u{index:03}.service"))
            .collect::<Vec<_>>()
            .join(" ");
        let root = record("root.service", "child.service", "", false)
            .replace("UpheldBy=", &format!("UpheldBy={references}"));
        let child = record("child.service", "", "", false);
        let session = Session {
            responses: Mutex::new(VecDeque::from([root.clone(), child, root])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.accounting.retained_edges, 1);
        assert_eq!(result.accounting.omitted_known_edges, 0);
        assert_eq!(result.accounting.retained_diagnostics, 512);
        assert_eq!(result.accounting.omitted_known_diagnostics, 1);
        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.conditional_diagnostics.len(), MAX_DIAGNOSTICS);
        assert!(
            result
                .warnings
                .contains(&StopImpactWarning::DiagnosticLimit)
        );
        assert!(!result.warnings.contains(&StopImpactWarning::EdgeLimit));
    }

    // 64 units, 7,040 raw passive references, 6,720 canonical diagnostics.
    // Every traversal response fits 64 KiB; the aggregate fits 256 KiB.
    fn large_diagnostic_responses(permuted: bool) -> Vec<String> {
        let children = (0..63)
            .map(|index| format!("child-{index:02}.service"))
            .collect::<Vec<_>>();
        let mut references = (0..20)
            .map(|index| format!("event-{index:02}.service"))
            .collect::<Vec<_>>();
        references.extend(["child-alias.service".into(), "child-00.service".into()]);
        if permuted {
            references.reverse();
        }
        let decorate = |mut output: String| {
            for property in [
                "OnSuccess",
                "OnFailure",
                "Triggers",
                "TriggeredBy",
                "UpheldBy",
            ] {
                output = output.replace(
                    &format!("{property}="),
                    &format!("{property}={}", references.join(" ")),
                );
            }
            if permuted {
                output.lines().rev().collect::<Vec<_>>().join("\n")
            } else {
                output
            }
        };
        let mut direct = children.clone();
        if permuted {
            direct.reverse();
        }
        let root = decorate(
            record("root-alias.service", &direct.join(" "), "", false)
                .replace(
                    "Names=root-alias.service",
                    "Names=root.service root-alias.service",
                )
                .replace("Following=", "Following=root.service"),
        );
        let mut responses = vec![root.clone()];
        responses.extend(children.chunks(16).map(|batch| {
            let mut outputs = batch
                .iter()
                .map(|child| {
                    let output = if child == "child-00.service" {
                        record("child-alias.service", "", "", false)
                            .replace(
                                "Names=child-alias.service",
                                if permuted {
                                    "Names=child-alias.service child-00.service"
                                } else {
                                    "Names=child-00.service child-alias.service"
                                },
                            )
                            .replace("Following=", "Following=child-00.service")
                    } else {
                        record(child, "", "", false)
                    };
                    decorate(output)
                })
                .collect::<Vec<_>>();
            if permuted {
                outputs.reverse();
            }
            outputs.join("\n\n")
        }));
        responses.push(root);
        assert_eq!(responses.len(), usize::from(MAX_QUERIES));
        assert!(responses.iter().all(|output| output.len() <= 64 * 1024));
        assert!(responses.iter().map(String::len).sum::<usize>() <= MAX_AGGREGATE_OUTPUT_BYTES);
        for output in &responses {
            assert!(
                output
                    .split("\n\n")
                    .all(|block| block.len() <= MAX_BLOCK_BYTES)
            );
            assert!(
                output
                    .lines()
                    .filter_map(|line| line.split_once('='))
                    .all(|(_, value)| value.len() <= MAX_PROPERTY_BYTES)
            );
            assert!(parse_systemd_stop_impact(output).is_ok());
        }
        responses
    }

    #[tokio::test]
    async fn large_diagnostic_assessment_is_exact_deterministic_and_ends_with_sixth_root_query() {
        let mut results = Vec::new();
        let mut requests = Vec::new();
        for permuted in [false, true] {
            let commands = Arc::new(Mutex::new(Vec::new()));
            let session = Session {
                responses: Mutex::new(VecDeque::from(large_diagnostic_responses(permuted))),
                commands: commands.clone(),
            };
            let result = assess_systemd_stop_impact(
                &session,
                CancellationToken::new(),
                HostId::new(),
                HostSessionId::new(),
                "root-alias.service".into(),
            )
            .await
            .unwrap();
            assert_eq!(result.accounting.retained_units, 64);
            assert_eq!(result.accounting.retained_diagnostics, 512);
            assert_eq!(result.accounting.omitted_known_diagnostics, 6208);
            assert_eq!(result.accounting.retained_edges, 63);
            assert_eq!(result.accounting.omitted_known_edges, 0);
            assert_eq!(result.accounting.actual_ssh_queries, 6);
            assert_eq!(result.completeness, StopImpactCompleteness::Partial);
            assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
            assert!(result.root_consistent);
            assert_eq!(result.warnings, [StopImpactWarning::DiagnosticLimit]);
            let related = std::iter::once("child-00.service".to_owned())
                .chain((0..20).map(|index| format!("event-{index:02}.service")))
                .collect::<Vec<_>>();
            let expected = (0..5)
                .flat_map(|index| {
                    let related = &related;
                    [
                        StopImpactDiagnosticKind::OnSuccessActivation,
                        StopImpactDiagnosticKind::OnFailureActivation,
                        StopImpactDiagnosticKind::Trigger,
                        StopImpactDiagnosticKind::TriggeredBy,
                        StopImpactDiagnosticKind::UpheldByReactivation,
                    ]
                    .into_iter()
                    .flat_map(move |kind| {
                        related.iter().map(move |name| SystemdStopImpactDiagnostic {
                            canonical_unit: format!("child-{index:02}.service"),
                            kind,
                            related_unit: Some(name.clone()),
                            job_mode: None,
                            manager_action: None,
                        })
                    })
                })
                .take(MAX_DIAGNOSTICS)
                .collect::<Vec<_>>();
            assert_eq!(result.conditional_diagnostics, expected);
            assert!(result.conditional_diagnostics.iter().all(|diagnostic| {
                diagnostic.canonical_unit != "child-alias.service"
                    && diagnostic.related_unit.as_deref() != Some("child-alias.service")
            }));
            let commands = commands.lock().unwrap().clone();
            assert_eq!(commands.len(), 6);
            assert_eq!(commands.first(), commands.last());
            assert!(
                commands
                    .last()
                    .unwrap()
                    .ends_with(" show -- 'root-alias.service'")
            );
            assert!(session.responses.lock().unwrap().is_empty());
            requests.push(commands);
            results.push(result);
        }
        assert_eq!(requests[0], requests[1]);
        assert_eq!(
            results[0].conditional_diagnostics,
            results[1].conditional_diagnostics
        );
        assert_eq!(results[0].accounting, results[1].accounting);
        assert_eq!(results[0].units, results[1].units);
        assert_eq!(results[0].edges, results[1].edges);
    }

    #[test]
    fn large_diagnostic_projection_bounds_every_intermediate_allocation_and_reserves_working_set() {
        for permuted in [false, true] {
            let responses = large_diagnostic_responses(permuted);
            let root = parse_systemd_stop_impact(&responses[0]).unwrap().remove(0);
            let mut builder = Builder::new(
                HostId::new(),
                HostSessionId::new(),
                "root-alias.service".into(),
                root,
                responses[0].len(),
            );
            builder.inspect_record("root.service");
            assert!(builder.enforce_working_set());
            for output in &responses[1..5] {
                let batch = builder.take_batch();
                builder
                    .retain_batch(batch, parse_systemd_stop_impact(output).unwrap())
                    .unwrap();
                assert!(builder.enforce_working_set());
                assert!(builder.working_set_bytes() <= MAX_WORKING_SET_BYTES);
            }
            let mut projection = DiagnosticProjection::new();
            let mut records = builder.records.values().collect::<Vec<_>>();
            records.sort_by(|left, right| left.record.id.cmp(&right.record.id));
            if permuted {
                records.reverse();
            }
            for retained in records {
                projection.project_record(
                    &retained.record,
                    &builder.aliases,
                    &builder.ambiguous_aliases,
                );
            }
            assert_eq!(projection.unique_count, 6720);
            assert_eq!(projection.peak_retained, 512);
            assert_eq!(projection.peak_seen_related, 21);
            assert_eq!(projection.retained.capacity(), 512);
            assert_eq!(projection.seen_related.capacity(), MAX_PROPERTY_BYTES);
            let owned_bytes = projection.retained.capacity()
                * size_of::<SystemdStopImpactDiagnostic>()
                + projection
                    .retained
                    .iter()
                    .map(|diagnostic| {
                        diagnostic.canonical_unit.capacity()
                            + diagnostic.related_unit.as_ref().map_or(0, String::capacity)
                    })
                    .sum::<usize>();
            let scratch_bytes = projection.seen_related.capacity() * size_of::<&str>();
            assert!(owned_bytes + scratch_bytes < DIAGNOSTIC_WORKING_SET_BYTES);
            assert!(
                builder.working_set_bytes() - DIAGNOSTIC_WORKING_SET_BYTES
                    + owned_bytes
                    + scratch_bytes
                    <= MAX_WORKING_SET_BYTES
            );
            drop(projection);

            // The old estimate alone would admit this state. The projection
            // reservation must reject it before finish allocates any diagnostics.
            let without_projection = builder.working_set_bytes() - DIAGNOSTIC_WORKING_SET_BYTES;
            let additional = (MAX_WORKING_SET_BYTES - without_projection) / (256 + 2 * 255);
            for index in 0..additional {
                let name = format!("{index:08}{}", "a".repeat(247));
                builder.aliases.insert(name.clone(), name);
            }
            assert!(
                builder.working_set_bytes() - DIAGNOSTIC_WORKING_SET_BYTES <= MAX_WORKING_SET_BYTES
            );
            assert!(builder.working_set_bytes() > MAX_WORKING_SET_BYTES);
            assert!(!builder.enforce_working_set());
            assert!(builder.incomplete);
            assert!(builder.unknown);
        }
    }

    #[test]
    fn diagnostic_projection_preserves_ambiguous_aliases_and_scalar_kinds() {
        let root = record("root.service", "", "", false)
            .replace("UpheldBy=", "UpheldBy=alias.service canonical.service")
            .replace("OnSuccessJobMode=fail", "OnSuccessJobMode=replace")
            .replace("OnFailureJobMode=replace", "OnFailureJobMode=fail")
            .replace("SuccessAction=none", "SuccessAction=reboot")
            .replace("FailureAction=none", "FailureAction=poweroff");
        let parsed = parse_systemd_stop_impact(&root).unwrap().remove(0);
        let aliases = HashMap::from([("alias.service".into(), "canonical.service".into())]);
        let ambiguous = HashSet::from(["alias.service".into()]);
        let mut projection = DiagnosticProjection::new();
        projection.project_record(&parsed, &aliases, &ambiguous);
        assert_eq!(projection.unique_count, 6);
        assert_eq!(projection.retained.len(), 6);
        assert!(
            projection
                .retained
                .iter()
                .any(|diagnostic| diagnostic.related_unit.as_deref() == Some("alias.service"))
        );
        let mut normalized = DiagnosticProjection::new();
        normalized.project_record(&parsed, &aliases, &HashSet::new());
        assert_eq!(normalized.unique_count, 5);
        assert_eq!(normalized.retained.len(), 5);
    }

    #[test]
    fn diagnostic_projection_reservation_covers_maximum_length_owned_identities() {
        let references = (0..32)
            .map(|index| format!("{index:02}{}.service", "r".repeat(245)))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(references.len(), MAX_PROPERTY_BYTES - 1);
        let records = (0..9)
            .map(|index| {
                let id = format!("{index:02}{}.service", "u".repeat(245));
                assert_eq!(id.len(), 255);
                let output = record(&id, "", "", false)
                    .replace("OnSuccess=", &format!("OnSuccess={references}"))
                    .replace("OnFailure=", &format!("OnFailure={references}"))
                    .replace("UpheldBy=", &format!("UpheldBy={references}"));
                assert!(output.len() <= MAX_BLOCK_BYTES);
                parse_systemd_stop_impact(&output).unwrap().remove(0)
            })
            .collect::<Vec<_>>();
        let aliases = HashMap::new();
        let ambiguous = HashSet::new();
        let mut projection = DiagnosticProjection::new();
        for record in records.iter().rev() {
            projection.project_record(record, &aliases, &ambiguous);
        }
        assert_eq!(projection.unique_count, 864);
        assert_eq!(projection.peak_retained, 512);
        assert_eq!(projection.peak_seen_related, 32);
        let maximum_owned_bytes =
            MAX_DIAGNOSTICS * (size_of::<SystemdStopImpactDiagnostic>() + 2 * 255);
        let scratch_bytes = projection.seen_related.capacity() * size_of::<&str>();
        assert!(
            maximum_owned_bytes + scratch_bytes + size_of::<DiagnosticProjection<'_>>()
                < DIAGNOSTIC_WORKING_SET_BYTES
        );
        assert!(projection.retained.iter().all(|diagnostic| {
            diagnostic.canonical_unit.capacity() == 255
                && diagnostic.related_unit.as_ref().unwrap().capacity() == 255
        }));
    }

    #[tokio::test]
    async fn malformed_final_root_is_partial_unknown_instead_of_aborting_assessment() {
        let root = record("root.service", "", "", false);
        let session = Session {
            responses: Mutex::new(VecDeque::from([root, "Id=malformed.service".into()])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert!(!result.root_consistent);
    }

    #[tokio::test]
    async fn query_frontier_is_bounded_and_final_query_remains_root_revalidation() {
        let names = (0..65)
            .map(|index| format!("child-{index:02}.service"))
            .collect::<Vec<_>>();
        let root = record("root.service", &names.join(" "), "", false);
        let responses = (0..4)
            .map(|batch| {
                names[batch * 16..(batch + 1) * 16]
                    .iter()
                    .map(|name| record(name, "", "", false))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .collect::<Vec<_>>();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            responses: Mutex::new(VecDeque::from([
                root.clone(),
                responses[0].clone(),
                responses[1].clone(),
                responses[2].clone(),
                responses[3].clone(),
                root,
            ])),
            commands: commands.clone(),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(result.accounting.actual_ssh_queries, 6);
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert!(result.accounting.unresolved_frontier_references > 0);
        let commands = commands.lock().unwrap();
        assert_eq!(commands.len(), 6);
        assert!(
            commands
                .last()
                .unwrap()
                .ends_with(" show -- 'root.service'")
        );
    }

    #[tokio::test]
    async fn frontier_order_is_stable_across_input_permutations_when_budget_is_exhausted() {
        let names = (0..65)
            .map(|index| format!("child-{index:02}.service"))
            .collect::<Vec<_>>();
        let sorted_outputs = names[..64]
            .chunks(16)
            .map(|batch| {
                batch
                    .iter()
                    .map(|name| record(name, "", "", false))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .collect::<Vec<_>>();
        let ordered_root = record(
            "root.service",
            &names.join(" "),
            "conditional.service",
            false,
        );
        let mut reversed = names.clone();
        reversed.reverse();
        let reversed_root = record(
            "root.service",
            &reversed.join(" "),
            "conditional.service",
            false,
        );

        let run = |root: String| {
            let commands = Arc::new(Mutex::new(Vec::new()));
            let session = Session {
                responses: Mutex::new(VecDeque::from([
                    root.clone(),
                    sorted_outputs[0].clone(),
                    sorted_outputs[1].clone(),
                    sorted_outputs[2].clone(),
                    sorted_outputs[3].clone(),
                    root,
                ])),
                commands: commands.clone(),
            };
            (session, commands)
        };
        let (ordered_session, ordered_commands) = run(ordered_root);
        let (reversed_session, reversed_commands) = run(reversed_root);
        let ordered = assess_systemd_stop_impact(
            &ordered_session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        let reversed = assess_systemd_stop_impact(
            &reversed_session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            *ordered_commands.lock().unwrap(),
            *reversed_commands.lock().unwrap()
        );
        assert_eq!(ordered.accounting.actual_ssh_queries, 6);
        assert_eq!(reversed.accounting.actual_ssh_queries, 6);
        assert!(
            ordered_commands
                .lock()
                .unwrap()
                .iter()
                .take(5)
                .all(|command| !command.contains("conditional.service"))
        );
    }

    #[tokio::test]
    async fn changed_root_revalidation_is_partial_and_unknown() {
        let root = record("root.service", "", "", false);
        let changed = root.replace("ActiveState=active", "ActiveState=inactive");
        let session = Session {
            responses: Mutex::new(VecDeque::from([root, changed])),
            commands: Arc::new(Mutex::new(Vec::new())),
        };
        let result = assess_systemd_stop_impact(
            &session,
            CancellationToken::new(),
            HostId::new(),
            HostSessionId::new(),
            "root.service".into(),
        )
        .await
        .unwrap();
        assert!(!result.root_consistent);
        assert_eq!(result.completeness, StopImpactCompleteness::Partial);
        assert_eq!(result.uncertainty, StopImpactUncertainty::UnknownImpact);
        assert!(
            result
                .warnings
                .contains(&StopImpactWarning::ConcurrentTopologyChange)
        );
    }
}
