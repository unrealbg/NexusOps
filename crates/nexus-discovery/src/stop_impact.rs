use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, StopImpactCompleteness, StopImpactProvenance,
    StopImpactRelationship, StopImpactUncertainty, StopImpactWarning, SystemdStopImpactAccounting,
    SystemdStopImpactAssessment, SystemdStopImpactEdge, SystemdStopImpactUnit,
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
const MAX_DEPTH: u8 = 4;
const MAX_ALIASES: usize = 16;
const MAX_PROPERTY_BYTES: usize = 8 * 1024;
const MAX_BLOCK_BYTES: usize = 32 * 1024;
const MAX_AGGREGATE_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_WORKING_SET_BYTES: usize = 2 * 1024 * 1024;
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
            || self.on_success_job_mode != "fail"
            || self.on_failure_job_mode != "replace"
            || self.success_action != "none"
            || self.failure_action != "none"
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
            on_success_job_mode: token(fields["OnSuccessJobMode"], 64)?,
            on_failure_job_mode: token(fields["OnFailureJobMode"], 64)?,
            success_action: token(fields["SuccessAction"], 64)?,
            failure_action: token(fields["FailureAction"], 64)?,
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
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .map(|_| true)
        .ok_or_else(invalid)
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
    #[derive(Clone, Copy)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut started = false;
    for character in value.chars() {
        if escaped {
            // Preserve systemd's `\\xHH` unit-name encoding for the typed
            // identity validator rather than interpreting shell escapes here.
            current.push('\\');
            current.push(character);
            escaped = false;
            started = true;
            continue;
        }
        match quote {
            Quote::None if character.is_ascii_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            Quote::None if character == '\'' => {
                quote = Quote::Single;
                started = true;
            }
            Quote::None if character == '"' => {
                quote = Quote::Double;
                started = true;
            }
            Quote::Single if character == '\'' => quote = Quote::None,
            Quote::Double if character == '"' => quote = Quote::None,
            Quote::None | Quote::Double if character == '\\' => {
                escaped = true;
                started = true;
            }
            _ => {
                current.push(character);
                started = true;
            }
        }
    }
    if escaped || !matches!(quote, Quote::None) {
        return Err(invalid());
    }
    if started {
        words.push(current);
    }
    if words.iter().any(String::is_empty) {
        return Err(invalid());
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
}

#[derive(Clone)]
struct Retained {
    record: ParsedStopImpactUnit,
    depth: u8,
    provenance: StopImpactProvenance,
    candidate: bool,
}

struct Builder {
    host_id: HostId,
    host_session_id: HostSessionId,
    root_initial: ParsedStopImpactUnit,
    records: HashMap<String, Retained>,
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
                omitted_known_units: 0,
                omitted_known_edges: 0,
                omitted_known_candidates: 0,
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

    fn enforce_working_set(&mut self) -> bool {
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
        let estimate = 64usize
            .saturating_mul(1024)
            .saturating_add(record_bytes)
            .saturating_add(edge_bytes)
            .saturating_add(alias_bytes)
            .saturating_add(pending_bytes);
        if estimate <= MAX_WORKING_SET_BYTES {
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
            for property in CANDIDATE_FORWARD {
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
            self.admit_query(target, depth, provenance, candidate, None);
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
        self.admit_query(&canonical_target, depth, provenance, candidate, None);
    }

    fn admit_query(
        &mut self,
        requested: &str,
        depth: u8,
        provenance: StopImpactProvenance,
        candidate: bool,
        candidate_source: Option<&str>,
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
            existing.depth = existing.depth.min(depth);
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
        (0..16).filter_map(|_| self.pending.pop_front()).collect()
    }

    fn retain_batch(
        &mut self,
        requested: Vec<Pending>,
        parsed: Vec<ParsedStopImpactUnit>,
    ) -> Result<(), AppError> {
        if requested.len() != parsed.len() {
            return Err(invalid());
        }
        let mut inspect = Vec::new();
        for (pending, record) in requested.into_iter().zip(parsed) {
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
            if self.records.len() >= MAX_NODES {
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
            if pending.candidate
                && !candidate_affected(
                    &record,
                    &self.records,
                    &self.aliases,
                    &self.ambiguous_aliases,
                )
            {
                self.accounting.omitted_known_candidates =
                    self.accounting.omitted_known_candidates.saturating_add(1);
                continue;
            }
            if pending.candidate {
                self.conditional = true;
                self.accounting.retained_candidates =
                    self.accounting.retained_candidates.saturating_add(1);
                for source in pending.candidate_sources {
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
                            source,
                            target: canonical.clone(),
                            relationship: StopImpactRelationship::StopWhenUnneededCandidate,
                            provenance: StopImpactProvenance::Conditional,
                        },
                    );
                }
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
        Ok(())
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
        if self.incomplete {
            self.unknown = true;
        }
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
        self.accounting.retained_units = u32::try_from(units.len()).unwrap_or(u32::MAX);
        self.accounting.retained_edges = u32::try_from(edges.len()).unwrap_or(u32::MAX);
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
            accounting: self.accounting,
            warnings,
            limitations: vec![
                "This diagnostic covers systemd-managed topology only; application dependencies, stop hooks, process ownership, and SSH, VPN, or network access effects may be unknown.".into(),
                "No dependency relationship is evidence that stopping a unit is safe.".into(),
            ],
        }
    }
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
                let parsed = parse_systemd_stop_impact(&output)?;
                builder.retain_batch(batch, parsed)?;
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
            let mut parsed = parse_systemd_stop_impact(&output)?;
            if parsed.len() == 1 {
                Some(parsed.remove(0))
            } else {
                None
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
        StopImpactWarning::ConcurrentTopologyChange => 8,
        StopImpactWarning::CandidateCoverageIncomplete => 9,
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
        8 => StopImpactWarning::ConcurrentTopologyChange,
        9 => StopImpactWarning::CandidateCoverageIncomplete,
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

    struct Session {
        responses: Mutex<VecDeque<String>>,
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

    #[tokio::test]
    async fn schedules_nonempty_batches_and_final_root_query_last() {
        let root = record("root.service", "child.service", "helper.service", false);
        let child = record("child.service", "", "", false);
        let helper = record("helper.service", "", "", true);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            responses: Mutex::new(VecDeque::from([
                root.clone(),
                [child, helper].join("\n\n"),
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
        assert!(result.root_consistent);
        assert_eq!(result.accounting.actual_ssh_queries, 3);
        assert_eq!(commands.lock().unwrap().len(), 3);
        assert!(commands.lock().unwrap()[2].ends_with(" show -- 'root.service'"));
    }

    #[tokio::test]
    async fn upgrades_cached_conditional_provenance_without_duplicate_query() {
        let root = record("root.service", "child.service", "helper.service", false);
        let child = record("child.service", "helper.service", "", false);
        let helper = record("helper.service", "root.service child.service", "", true);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            responses: Mutex::new(VecDeque::from([
                root.clone(),
                [child, helper].join("\n\n"),
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
        let helper = result
            .units
            .iter()
            .find(|unit| unit.canonical_unit == "helper.service")
            .unwrap();
        assert_eq!(helper.provenance, StopImpactProvenance::Direct);
        assert_eq!(result.accounting.actual_ssh_queries, 3);
        assert_eq!(commands.lock().unwrap().len(), 3);
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
    }

    #[tokio::test]
    async fn query_frontier_is_bounded_and_final_query_remains_root_revalidation() {
        let names = (0..65)
            .map(|index| format!("child-{index}.service"))
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
