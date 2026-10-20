use super::*;
use async_trait::async_trait;
use std::sync::Mutex;

const ROOT: &str = "synthetic-root.service";
const HELPER: &str = "synthetic-helper.service";
const SLICE: &str = "synthetic-slice.slice";
const ALIAS: &str = "synthetic-slice-alias.slice";

fn output(id: &str, properties: &[(&str, &str)]) -> String {
    nexus_operations::SYSTEMD_STOP_IMPACT_PROPERTIES
        .iter()
        .map(|property| {
            let default = match *property {
                "Id" | "Names" => id,
                "LoadState" => "loaded",
                "ActiveState" => "active",
                "SubState" => "running",
                "CanStop" => "yes",
                "RefuseManualStop" | "NeedDaemonReload" | "StopWhenUnneeded" => "no",
                "OnSuccessJobMode" => "fail",
                "OnFailureJobMode" => "replace",
                "SuccessAction" | "FailureAction" => "none",
                _ => "",
            };
            let value = properties
                .iter()
                .find(|(key, _)| key == property)
                .map_or(default, |(_, value)| *value);
            format!("{property}={value}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Session {
    records: BTreeMap<String, String>,
    reverse_response: bool,
    queries: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl RemoteSession for Session {
    async fn execute(
        &self,
        _: nexus_operations::ReadOnlyCommand,
        _: CancellationToken,
    ) -> Result<String, AppError> {
        panic!("Only typed stop-impact reads are allowed")
    }

    async fn execute_stop_impact(
        &self,
        query: &SystemdStopImpactQuery,
        _: CancellationToken,
    ) -> Result<String, AppError> {
        self.queries.lock().unwrap().push(query.units().to_vec());
        let mut response = query
            .units()
            .iter()
            .map(|name| self.records[name].clone())
            .collect::<Vec<_>>();
        if self.reverse_response {
            response.reverse();
        }
        Ok(response.join("\n\n"))
    }

    async fn disconnect(&self) -> Result<(), AppError> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

async fn run(
    records: BTreeMap<String, String>,
    reverse_response: bool,
) -> (SystemdStopImpactAssessment, Vec<Vec<String>>) {
    let session = Session {
        records,
        reverse_response,
        queries: Mutex::default(),
    };
    let assessment = assess_systemd_stop_impact(
        &session,
        CancellationToken::new(),
        HostId::new(),
        HostSessionId::new(),
        ROOT.into(),
    )
    .await
    .unwrap();
    let queries = session.queries.into_inner().unwrap();
    let mut observed = BTreeSet::new();
    for name in queries.iter().take(queries.len() - 1).flatten() {
        let canonical = parse_systemd_stop_impact(&session.records[name])
            .unwrap()
            .remove(0)
            .id;
        assert!(
            observed.insert(canonical),
            "Duplicate canonical query: {name}"
        );
    }
    assert_eq!(queries.last().unwrap(), &[ROOT]);
    assert!(assessment.root_consistent);
    assert!(queries.len() <= 6);
    assert_eq!(
        assessment.accounting.actual_ssh_queries as usize,
        queries.len()
    );
    assert_eq!(
        assessment.accounting.retained_units as usize,
        assessment.units.len()
    );
    assert_eq!(
        assessment.accounting.retained_edges as usize,
        assessment.edges.len()
    );
    assert_paths(&assessment);
    (assessment, queries)
}

// Independent DTO closure: candidate probes are not propagation edges;
// conditional paths need a validated candidate seed reachable from the root.
fn assert_paths(assessment: &SystemdStopImpactAssessment) {
    let mut direct = BTreeSet::from([assessment.root_unit.as_str()]);
    let mut conditional = BTreeSet::new();
    loop {
        let before = (direct.len(), conditional.len());
        for edge in &assessment.edges {
            if edge.provenance == StopImpactProvenance::Direct
                && edge.relationship != StopImpactRelationship::StopWhenUnneededCandidate
                && direct.contains(edge.source.as_str())
            {
                direct.insert(edge.target.as_str());
            }
            if edge.provenance == StopImpactProvenance::Conditional
                && ((edge.relationship == StopImpactRelationship::StopWhenUnneededCandidate
                    && (direct.contains(edge.source.as_str())
                        || conditional.contains(edge.source.as_str())))
                    || (edge.relationship != StopImpactRelationship::StopWhenUnneededCandidate
                        && conditional.contains(edge.source.as_str())))
            {
                conditional.insert(edge.target.as_str());
            }
        }
        if before == (direct.len(), conditional.len()) {
            break;
        }
    }
    for unit in &assessment.units {
        let reachable = match unit.provenance {
            StopImpactProvenance::Direct => &direct,
            StopImpactProvenance::Conditional => &conditional,
        };
        assert!(reachable.contains(unit.canonical_unit.as_str()), "{unit:?}");
    }
}

fn provenance(assessment: &SystemdStopImpactAssessment, name: &str) -> StopImpactProvenance {
    assessment
        .units
        .iter()
        .find(|unit| unit.canonical_unit == name)
        .unwrap()
        .provenance
}

fn repeated(swu: bool, alias: bool, reversed: bool) -> BTreeMap<String, String> {
    let requested = if alias { ALIAS } else { SLICE };
    let requires = if reversed {
        format!("{requested} {HELPER}")
    } else {
        format!("{HELPER} {requested}")
    };
    let reverse = if reversed {
        format!("{HELPER} {ROOT}")
    } else {
        format!("{ROOT} {HELPER}")
    };
    BTreeMap::from([
        (ROOT.into(), output(ROOT, &[("Requires", &requires)])),
        (
            HELPER.into(),
            output(
                HELPER,
                &[
                    ("StopWhenUnneeded", "yes"),
                    ("RequiredBy", ROOT),
                    ("Requires", SLICE),
                ],
            ),
        ),
        (
            requested.into(),
            output(
                SLICE,
                &[
                    ("Names", &format!("{SLICE} {ALIAS}")),
                    ("RequiredBy", &reverse),
                    ("StopWhenUnneeded", if swu { "yes" } else { "no" }),
                ],
            ),
        ),
    ])
}

async fn check_repeated(swu: bool) {
    let mut previous: Option<SystemdStopImpactAssessment> = None;
    for alias in [false, true] {
        for reversed in [false, true] {
            let (assessment, queries) = run(repeated(swu, alias, reversed), reversed).await;
            let accounting = &assessment.accounting;
            assert_eq!(queries.len(), 3);
            assert_eq!(accounting.observed_unit_records, 3);
            assert_eq!(accounting.observed_candidate_references, 3);
            assert_eq!(accounting.retained_candidates, if swu { 2 } else { 1 });
            assert_eq!(accounting.omitted_known_candidates, if swu { 0 } else { 1 });
            assert_eq!(accounting.retained_units, if swu { 3 } else { 2 });
            assert_eq!(accounting.retained_edges, if swu { 6 } else { 2 });
            assert_eq!(
                accounting.observed_relationship_references,
                if swu { 6 } else { 4 }
            );
            assert_eq!(accounting.omitted_known_units, 0);
            assert_eq!(accounting.omitted_known_edges, 0);
            assert_eq!(accounting.unresolved_frontier_references, 0);
            assert_eq!(assessment.completeness, StopImpactCompleteness::Complete);
            assert_eq!(
                assessment.uncertainty,
                StopImpactUncertainty::ConditionalImpact
            );
            assert!(assessment.warnings.is_empty());
            let consequence = assessment
                .conditional_consequences
                .iter()
                .find(|item| item.canonical_unit == SLICE)
                .unwrap();
            assert_eq!(consequence.sources, [HELPER, ROOT]);
            assert_eq!(
                consequence.classification,
                if swu {
                    StopImpactConditionalClassification::ConditionallyAffected
                } else {
                    StopImpactConditionalClassification::NotStopWhenUnneeded
                }
            );
            assert_eq!(
                assessment
                    .units
                    .iter()
                    .any(|unit| unit.canonical_unit == SLICE),
                swu
            );
            assert!(
                assessment
                    .edges
                    .iter()
                    .all(|edge| edge.source != ALIAS && edge.target != ALIAS)
            );
            if swu {
                assert_eq!(
                    provenance(&assessment, SLICE),
                    StopImpactProvenance::Conditional
                );
                for source in [ROOT, HELPER] {
                    assert!(assessment.edges.iter().any(|edge| edge.source == source
                        && edge.target == SLICE
                        && edge.relationship == StopImpactRelationship::StopWhenUnneededCandidate
                        && edge.provenance == StopImpactProvenance::Conditional));
                }
            } else {
                assert!(!assessment.edges.iter().any(|edge| edge.target == SLICE));
            }
            if let Some(previous) = &previous {
                assert_eq!(previous.units, assessment.units);
                assert_eq!(previous.edges, assessment.edges);
                assert_eq!(
                    previous.conditional_consequences,
                    assessment.conditional_consequences
                );
                assert_eq!(
                    previous.conditional_diagnostics,
                    assessment.conditional_diagnostics
                );
                assert_eq!(previous.accounting, assessment.accounting);
            }
            previous = Some(assessment);
        }
    }
}

#[tokio::test]
async fn repeated_non_swu_candidate_is_consequence_only_with_merged_canonical_sources() {
    check_repeated(false).await;
}

#[tokio::test]
async fn repeated_swu_candidate_uses_eligibility_edges_and_exact_accounting() {
    check_repeated(true).await;
}

#[tokio::test]
async fn shared_sources_discovered_after_candidate_admission_are_not_lost() {
    for reversed in [false, true] {
        let records = BTreeMap::from([
            (
                ROOT.into(),
                output(ROOT, &[("Requires", "a-helper.service z-helper.service")]),
            ),
            (
                "a-helper.service".into(),
                output("a-helper.service", &[("StopWhenUnneeded", "yes")]),
            ),
            (
                "z-helper.service".into(),
                output(
                    "z-helper.service",
                    &[
                        ("StopWhenUnneeded", "yes"),
                        ("Requires", "a-helper.service"),
                        ("Wants", "a-helper.service"),
                    ],
                ),
            ),
        ]);
        let (assessment, _) = run(records, reversed).await;
        let consequence = assessment
            .conditional_consequences
            .iter()
            .find(|item| item.canonical_unit == "a-helper.service")
            .unwrap();
        assert_eq!(consequence.sources, [ROOT, "z-helper.service"]);
        assert_eq!(assessment.accounting.retained_candidates, 2);
        assert_eq!(assessment.accounting.retained_edges, 3);
        assert_eq!(assessment.accounting.observed_candidate_references, 4);
        assert_eq!(assessment.accounting.omitted_known_candidates, 0);
    }
}

#[tokio::test]
async fn conditional_relationship_admits_cached_non_swu_observation_and_descendant() {
    // The alias is queued while its canonical observation is still pending.
    // Depth ordering and the 16-unit batch limit keep the two out of one batch.
    let seed = "a-seed.service";
    let helper = "z-helper.service";
    let alias = "zz-helper-alias.service";
    let child = "zz-child.service";
    let mut names = (0..30)
        .map(|index| format!("m-{index:02}.service"))
        .collect::<Vec<_>>();
    names.extend([seed.into(), helper.into()]);
    let mut records = names
        .iter()
        .map(|name| (name.clone(), output(name, &[])))
        .collect::<BTreeMap<_, _>>();
    records.insert(ROOT.into(), output(ROOT, &[("Requires", &names.join(" "))]));
    records.insert(
        seed.into(),
        output(
            seed,
            &[
                ("StopWhenUnneeded", "yes"),
                ("RequiredBy", ROOT),
                ("PropagatesStopTo", alias),
            ],
        ),
    );
    let helper_output = output(
        helper,
        &[
            ("Names", &format!("{helper} {alias}")),
            ("PropagatesStopTo", child),
        ],
    );
    records.insert(helper.into(), helper_output.clone());
    records.insert(alias.into(), helper_output);
    records.insert(child.into(), output(child, &[]));
    let (assessment, queries) = run(records, true).await;
    assert_eq!(queries.len(), 5);
    assert_eq!(queries[1].len(), 16);
    assert_eq!(queries[2].len(), 16);
    assert_eq!(queries[3], [child]);
    assert_eq!(
        provenance(&assessment, helper),
        StopImpactProvenance::Conditional
    );
    assert_eq!(
        provenance(&assessment, child),
        StopImpactProvenance::Conditional
    );
    assert_eq!(assessment.accounting.observed_unit_records, 34);
    assert_eq!(assessment.accounting.retained_units, 4);
    assert_eq!(assessment.accounting.retained_edges, 4);
    assert_eq!(assessment.accounting.retained_candidates, 1);
    assert_eq!(assessment.accounting.omitted_known_candidates, 30);
    assert_eq!(assessment.accounting.unresolved_frontier_references, 0);
    assert!(
        !assessment
            .conditional_consequences
            .iter()
            .any(|item| item.canonical_unit == helper || item.canonical_unit == child)
    );
    assert_eq!(assessment.completeness, StopImpactCompleteness::Complete);
    assert_eq!(
        assessment.uncertainty,
        StopImpactUncertainty::ConditionalImpact
    );
}

#[tokio::test]
async fn queued_candidate_alias_merges_sources_after_canonical_admission() {
    let seed = "a-seed.service";
    let helper = "z-helper.service";
    let alias = "zz-helper-alias.service";
    let mut names = (0..30)
        .map(|index| format!("m-{index:02}.service"))
        .collect::<Vec<_>>();
    names.extend([seed.into(), helper.into()]);
    let mut records = names
        .iter()
        .map(|name| (name.clone(), output(name, &[])))
        .collect::<BTreeMap<_, _>>();
    records.insert(ROOT.into(), output(ROOT, &[("Requires", &names.join(" "))]));
    records.insert(
        seed.into(),
        output(
            seed,
            &[
                ("StopWhenUnneeded", "yes"),
                ("RequiredBy", ROOT),
                ("Requires", alias),
            ],
        ),
    );
    let helper_output = output(
        helper,
        &[
            ("StopWhenUnneeded", "yes"),
            ("Names", &format!("{helper} {alias}")),
            ("RequiredBy", &format!("{ROOT} {seed}")),
        ],
    );
    records.insert(helper.into(), helper_output.clone());
    records.insert(alias.into(), helper_output);
    let (assessment, queries) = run(records, true).await;
    assert_eq!(queries.len(), 4);
    let consequence = assessment
        .conditional_consequences
        .iter()
        .find(|item| item.canonical_unit == helper)
        .unwrap();
    assert_eq!(consequence.sources, [seed, ROOT]);
    assert_eq!(assessment.accounting.retained_candidates, 2);
    assert_eq!(assessment.accounting.retained_edges, 6);
    assert_eq!(assessment.accounting.omitted_known_candidates, 30);
    assert_eq!(assessment.accounting.observed_unit_records, 33);
    assert_eq!(assessment.accounting.retained_units, 3);
}

fn parsed(id: &str, properties: &[(&str, &str)]) -> ParsedStopImpactUnit {
    parse_systemd_stop_impact(&output(id, properties))
        .unwrap()
        .remove(0)
}

#[tokio::test]
async fn delayed_alias_cannot_fabricate_direct_promotion_of_another_conditional_branch() {
    // On the reviewed source the delayed alias fabricated Direct at the cache
    // transition, upgraded the branch edge, then retained the in-flight child
    // as Conditional. The fix preserves the incoming Conditional throughout.
    let seed = "a-seed.service";
    let branch = "b-branch.service";
    let helper = "z-helper.service";
    let alias = "a-helper-alias.service";
    let child = "zz-child.service";
    let mut names = (0..29)
        .map(|index| format!("m-{index:02}.service"))
        .collect::<Vec<_>>();
    names.extend([seed.into(), branch.into(), helper.into()]);
    let mut records = names
        .iter()
        .map(|name| (name.clone(), output(name, &[])))
        .collect::<BTreeMap<_, _>>();
    records.insert(ROOT.into(), output(ROOT, &[("Requires", &names.join(" "))]));
    records.insert(
        seed.into(),
        output(
            seed,
            &[("StopWhenUnneeded", "yes"), ("PropagatesStopTo", alias)],
        ),
    );
    records.insert(
        branch.into(),
        output(
            branch,
            &[("StopWhenUnneeded", "yes"), ("PropagatesStopTo", child)],
        ),
    );
    let helper_output = output(
        helper,
        &[
            ("Names", &format!("{helper} {alias}")),
            ("PropagatesStopTo", branch),
        ],
    );
    records.insert(helper.into(), helper_output.clone());
    records.insert(alias.into(), helper_output);
    records.insert(child.into(), output(child, &[]));
    let (assessment, queries) = run(records, true).await;
    assert_eq!(queries.len(), 5);
    assert_eq!(queries[3], [child]);
    for name in [seed, branch, helper, child] {
        assert_eq!(
            provenance(&assessment, name),
            StopImpactProvenance::Conditional
        );
    }
    assert!(assessment.edges.iter().any(|edge| edge.source == branch
        && edge.target == child
        && edge.provenance == StopImpactProvenance::Conditional));
    assert_eq!(assessment.accounting.retained_candidates, 2);
    assert_eq!(assessment.accounting.omitted_known_candidates, 29);
    assert_eq!(assessment.accounting.retained_units, 5);
    assert_eq!(assessment.accounting.retained_edges, 5);
    assert_eq!(assessment.completeness, StopImpactCompleteness::Complete);
    assert_eq!(
        assessment.uncertainty,
        StopImpactUncertainty::ConditionalImpact
    );
}

#[test]
fn genuine_direct_discovery_of_cached_candidate_keeps_cached_descendants_direct() {
    // Controlled Builder transition: the public scheduler drains direct work
    // before querying candidates, so late direct discovery is not claimed reachable.
    let root = parsed(ROOT, &[]);
    let mut builder = Builder::new(
        HostId::new(),
        HostSessionId::new(),
        ROOT.into(),
        root.clone(),
        0,
    );
    builder.admit_query(
        HELPER,
        1,
        StopImpactProvenance::Conditional,
        true,
        Some(ROOT),
        5,
    );
    let batch = builder.take_batch();
    builder
        .retain_batch(batch, vec![parsed(HELPER, &[("PropagatesStopTo", SLICE)])])
        .unwrap();
    assert!(builder.candidates.contains_key(HELPER));
    builder.admit_edge_and_query(
        ROOT,
        HELPER,
        StopImpactRelationship::RequiredBy,
        1,
        StopImpactProvenance::Direct,
        false,
    );
    let batch = builder.take_batch();
    builder
        .retain_batch(batch, vec![parsed(SLICE, &[])])
        .unwrap();
    let assessment = builder.finish(Some(root));
    assert_eq!(
        provenance(&assessment, HELPER),
        StopImpactProvenance::Direct
    );
    assert_eq!(provenance(&assessment, SLICE), StopImpactProvenance::Direct);
    assert!(assessment.conditional_consequences.is_empty());
    assert_eq!(assessment.accounting.retained_candidates, 0);
    assert_paths(&assessment);
}

#[test]
fn genuine_direct_promotion_preserves_candidate_evidence_and_updates_cached_descendants() {
    let root = parsed(ROOT, &[]);
    let mut builder = Builder::new(
        HostId::new(),
        HostSessionId::new(),
        ROOT.into(),
        root.clone(),
        0,
    );
    builder.admit_query(
        HELPER,
        1,
        StopImpactProvenance::Conditional,
        true,
        Some(ROOT),
        5,
    );
    let batch = builder.take_batch();
    builder
        .retain_batch(
            batch,
            vec![parsed(
                HELPER,
                &[("StopWhenUnneeded", "yes"), ("PropagatesStopTo", SLICE)],
            )],
        )
        .unwrap();
    let batch = builder.take_batch();
    builder
        .retain_batch(batch, vec![parsed(SLICE, &[])])
        .unwrap();
    assert_eq!(
        builder.records[SLICE].provenance,
        StopImpactProvenance::Conditional
    );
    builder.admit_edge_and_query(
        ROOT,
        HELPER,
        StopImpactRelationship::RequiredBy,
        1,
        StopImpactProvenance::Direct,
        false,
    );
    let assessment = builder.finish(Some(root));
    assert_eq!(
        provenance(&assessment, HELPER),
        StopImpactProvenance::Direct
    );
    assert_eq!(provenance(&assessment, SLICE), StopImpactProvenance::Direct);
    assert_eq!(assessment.accounting.retained_candidates, 1);
    assert_eq!(assessment.conditional_consequences.len(), 1);
    assert!(assessment.edges.iter().any(|edge| edge.source == HELPER
        && edge.target == SLICE
        && edge.provenance == StopImpactProvenance::Direct));
    assert_paths(&assessment);
}

#[tokio::test]
async fn conditional_discovery_of_known_cached_candidate_does_not_strengthen_descendants() {
    let seed = "a-seed.service";
    let child = "synthetic-child.service";
    let records = BTreeMap::from([
        (
            ROOT.into(),
            output(ROOT, &[("Requires", &format!("{seed} {HELPER}"))]),
        ),
        (
            seed.into(),
            output(
                seed,
                &[
                    ("StopWhenUnneeded", "yes"),
                    ("RequiredBy", ROOT),
                    ("PropagatesStopTo", HELPER),
                ],
            ),
        ),
        (
            HELPER.into(),
            output(HELPER, &[("PropagatesStopTo", child)]),
        ),
        (child.into(), output(child, &[])),
    ]);
    let (assessment, queries) = run(records, false).await;
    assert_eq!(queries.len(), 4);
    assert_eq!(
        provenance(&assessment, HELPER),
        StopImpactProvenance::Conditional
    );
    assert_eq!(
        provenance(&assessment, child),
        StopImpactProvenance::Conditional
    );
    assert_eq!(assessment.accounting.retained_candidates, 1);
    assert_eq!(assessment.accounting.omitted_known_candidates, 0);
    assert_eq!(assessment.conditional_consequences.len(), 1);
    assert_eq!(assessment.accounting.retained_edges, 4);
}

#[tokio::test]
async fn node_and_query_exhaustion_keep_observation_and_retention_separate() {
    let names = (0..65)
        .map(|index| format!("synthetic-frontier-{index:02}.service"))
        .collect::<Vec<_>>();
    let mut records = names
        .iter()
        .map(|name| (name.clone(), output(name, &[])))
        .collect::<BTreeMap<_, _>>();
    records.insert(ROOT.into(), output(ROOT, &[("Requires", &names.join(" "))]));
    let (assessment, queries) = run(records, true).await;
    assert_eq!(queries.len(), 6);
    assert_eq!(assessment.accounting.observed_unit_records, 65);
    assert_eq!(assessment.accounting.retained_units, 1);
    assert_eq!(assessment.accounting.retained_edges, 0);
    assert_eq!(assessment.accounting.retained_candidates, 0);
    assert_eq!(assessment.accounting.omitted_known_units, 1);
    assert_eq!(assessment.accounting.omitted_known_candidates, 64);
    assert_eq!(assessment.accounting.unresolved_frontier_references, 1);
    assert_eq!(assessment.completeness, StopImpactCompleteness::Partial);
    assert_eq!(assessment.uncertainty, StopImpactUncertainty::UnknownImpact);
    for warning in [
        StopImpactWarning::NodeLimit,
        StopImpactWarning::QueryLimit,
        StopImpactWarning::CandidateCoverageIncomplete,
    ] {
        assert!(assessment.warnings.contains(&warning));
    }
}

#[tokio::test]
async fn depth_exhaustion_preserves_retained_paths_and_unqueried_frontier() {
    let mut records = BTreeMap::from([(
        ROOT.into(),
        output(ROOT, &[("RequiredBy", "depth-1.service")]),
    )]);
    for depth in 1..=5 {
        let name = format!("depth-{depth}.service");
        let next = format!("depth-{}.service", depth + 1);
        records.insert(name.clone(), output(&name, &[("RequiredBy", &next)]));
    }
    let (assessment, queries) = run(records, false).await;
    assert_eq!(queries.len(), 6);
    assert_eq!(assessment.accounting.retained_units, 5);
    assert_eq!(assessment.accounting.retained_edges, 5);
    assert_eq!(assessment.accounting.unresolved_frontier_references, 1);
    assert_eq!(assessment.completeness, StopImpactCompleteness::Partial);
    assert_eq!(assessment.uncertainty, StopImpactUncertainty::UnknownImpact);
    assert!(assessment.warnings.contains(&StopImpactWarning::DepthLimit));
}

#[test]
fn merged_candidate_sources_and_admitted_consequences_are_in_working_set_accounting() {
    let root = parsed(ROOT, &[]);
    let mut builder = Builder::new(HostId::new(), HostSessionId::new(), ROOT.into(), root, 0);
    builder.admit_query(
        HELPER,
        1,
        StopImpactProvenance::Conditional,
        true,
        Some(ROOT),
        5,
    );
    let batch = builder.take_batch();
    builder
        .retain_batch(batch, vec![parsed(HELPER, &[])])
        .unwrap();
    let before = builder.working_set_bytes();
    builder.admit_query(
        HELPER,
        2,
        StopImpactProvenance::Conditional,
        true,
        Some(SLICE),
        5,
    );
    assert_eq!(builder.working_set_bytes() - before, 128 + SLICE.len());
    assert_eq!(builder.candidates[HELPER].depth, 1);
    assert!(!builder.records.contains_key(HELPER));
    assert!(builder.enforce_working_set());
    builder.conditional_consequences.insert(
        HELPER.into(),
        (
            BTreeSet::from([ROOT.into(), SLICE.into()]),
            StopImpactConditionalClassification::ConditionallyAffected,
        ),
    );
    assert_eq!(
        builder.working_set_bytes() - before,
        128 + SLICE.len() + 512 + HELPER.len() + 256 + ROOT.len() + SLICE.len()
    );
    assert_eq!(MAX_WORKING_SET_BYTES, 2 * 1024 * 1024);
}
