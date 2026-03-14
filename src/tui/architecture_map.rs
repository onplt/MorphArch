use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;

use crate::config::{
    ClusterKindHint, ClusterKindMode, ClusteringConfig, ClusteringConstraintType,
    ClusteringStrategy,
};

const OVERVIEW_NODE_THRESHOLD: usize = 70;
const MIN_OVERVIEW_CLUSTERS: usize = 2;
const DEPS_NAMESPACE: &str = "deps";
const ROOT_NAMESPACE: &str = "__root__";

#[derive(Debug, Clone)]
pub struct ArchitectureMap {
    pub clusters: Vec<ClusterNode>,
    pub edges: Vec<ClusterEdge>,
    pub cluster_of_node: Vec<usize>,
    pub max_layer: usize,
    pub should_default_to_overview: bool,
}

#[derive(Debug, Clone)]
pub struct ClusterNode {
    pub id: usize,
    pub name: String,
    pub kind: ClusterKind,
    pub members: Vec<usize>,
    pub internal_member_count: usize,
    pub external_member_count: usize,
    pub anchor_label: String,
    pub layer: usize,
    pub x_ratio: f64,
    pub y_ratio: f64,
    pub inbound_weight: u32,
    pub outbound_weight: u32,
    pub internal_weight: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterKind {
    Workspace,
    Deps,
    Entry,
    External,
    Infra,
    Domain,
    Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClusterOverviewRole {
    PrimaryArchitecture,
    SupportCluster,
    ExternalSink,
}

#[derive(Debug, Clone)]
pub struct ClusterEdge {
    pub from: usize,
    pub to: usize,
    pub total_weight: u32,
    pub edge_count: usize,
}

impl ClusterNode {
    pub fn is_internal_bearing(&self) -> bool {
        self.internal_member_count > 0
    }

    pub fn is_external_only(&self) -> bool {
        self.internal_member_count == 0 && self.external_member_count > 0
    }

    pub fn is_dependency_sink(&self) -> bool {
        matches!(self.kind, ClusterKind::Deps) || self.is_external_only()
    }

    pub fn overview_role(&self) -> ClusterOverviewRole {
        if self.is_dependency_sink() {
            ClusterOverviewRole::ExternalSink
        } else if self.external_member_count > self.internal_member_count.saturating_mul(3)
            && self.internal_member_count <= 2
        {
            ClusterOverviewRole::SupportCluster
        } else {
            ClusterOverviewRole::PrimaryArchitecture
        }
    }
}

impl ArchitectureMap {
    pub fn build(
        labels: &[String],
        edges: &[(usize, usize)],
        edge_weights: &[u32],
        internal_nodes: Option<&HashSet<usize>>,
        config: &ClusteringConfig,
    ) -> Option<Self> {
        if labels.len() < 3 {
            return None;
        }

        let adjacency = build_undirected_adjacency(labels.len(), edges, edge_weights);
        let semantic = namespace_assignments(labels, internal_nodes, config);
        let mut cluster_of_node = match config.effective_strategy() {
            ClusteringStrategy::Namespace => semantic
                .as_ref()
                .map(|assignments| assignments.cluster_of_node.clone())
                .unwrap_or_else(|| label_propagation(&adjacency)),
            ClusteringStrategy::Structural => label_propagation(&adjacency),
            ClusteringStrategy::Hybrid => semantic
                .as_ref()
                .map(|assignments| {
                    let mut seeded = assignments.cluster_of_node.clone();
                    split_dominant_semantic_clusters(
                        &mut seeded,
                        assignments,
                        &adjacency,
                        labels,
                        internal_nodes,
                        labels.len(),
                        config,
                    );
                    seeded
                })
                .unwrap_or_else(|| label_propagation(&adjacency)),
        };
        apply_must_group_constraints(&mut cluster_of_node, labels, config);

        if config.structural_enabled() {
            merge_small_clusters(
                &mut cluster_of_node,
                labels,
                edges,
                edge_weights,
                internal_nodes,
                config,
                config.effective_min_cluster_size().max(2),
            );
            split_large_generic_clusters(
                &mut cluster_of_node,
                labels,
                edges,
                edge_weights,
                internal_nodes,
                config,
            );
            if config.post_merge_small_clusters() {
                merge_small_clusters(
                    &mut cluster_of_node,
                    labels,
                    edges,
                    edge_weights,
                    internal_nodes,
                    config,
                    config.effective_min_cluster_size().max(2),
                );
            }
        }
        apply_must_group_constraints(&mut cluster_of_node, labels, config);
        apply_must_separate_constraints(&mut cluster_of_node, labels, config);

        let mut members_by_cluster = HashMap::<usize, Vec<usize>>::new();
        for (node_idx, &cluster_id) in cluster_of_node.iter().enumerate() {
            members_by_cluster
                .entry(cluster_id)
                .or_default()
                .push(node_idx);
        }
        if members_by_cluster.len() < 2 {
            return None;
        }

        let mut canonical_ids = members_by_cluster.keys().copied().collect::<Vec<_>>();
        canonical_ids.sort_unstable();
        let id_remap = canonical_ids
            .iter()
            .enumerate()
            .map(|(new_id, old_id)| (*old_id, new_id))
            .collect::<HashMap<_, _>>();

        for cluster_id in &mut cluster_of_node {
            *cluster_id = id_remap[cluster_id];
        }

        let mut members = vec![Vec::<usize>::new(); canonical_ids.len()];
        for (node_idx, &cluster_id) in cluster_of_node.iter().enumerate() {
            members[cluster_id].push(node_idx);
        }

        let normalized = normalized_weights(edges.len(), edge_weights);
        let mut inbound_weight = vec![0u32; members.len()];
        let mut outbound_weight = vec![0u32; members.len()];
        let mut internal_weight = vec![0u32; members.len()];
        let mut aggregate = HashMap::<(usize, usize), (u32, usize)>::new();
        let mut node_degree = vec![0u32; labels.len()];

        for (edge_idx, &(from, to)) in edges.iter().enumerate() {
            if from >= labels.len() || to >= labels.len() {
                continue;
            }
            let weight = normalized[edge_idx];
            node_degree[from] += weight;
            node_degree[to] += weight;
            let from_cluster = cluster_of_node[from];
            let to_cluster = cluster_of_node[to];
            if from_cluster == to_cluster {
                internal_weight[from_cluster] += weight;
            } else {
                outbound_weight[from_cluster] += weight;
                inbound_weight[to_cluster] += weight;
                let entry = aggregate
                    .entry((from_cluster, to_cluster))
                    .or_insert((0, 0));
                entry.0 += weight;
                entry.1 += 1;
            }
        }

        let edges = aggregate
            .into_iter()
            .map(|((from, to), (total_weight, edge_count))| ClusterEdge {
                from,
                to,
                total_weight,
                edge_count,
            })
            .collect::<Vec<_>>();

        let layer_info = compute_cluster_layers(members.len(), &edges);
        let mut by_layer = vec![Vec::<usize>::new(); layer_info.max_layer + 1];
        for (cluster_id, &layer) in layer_info.layers.iter().enumerate() {
            by_layer[layer].push(cluster_id);
        }

        for layer_clusters in &mut by_layer {
            layer_clusters.sort_by(|a, b| {
                outbound_weight[*b]
                    .cmp(&outbound_weight[*a])
                    .then_with(|| inbound_weight[*b].cmp(&inbound_weight[*a]))
                    .then_with(|| members[*b].len().cmp(&members[*a].len()))
            });
        }

        let mut clusters = members
            .into_iter()
            .enumerate()
            .map(|(cluster_id, mut cluster_members)| {
                cluster_members.sort_by(|a, b| labels[*a].cmp(&labels[*b]).then_with(|| a.cmp(b)));
                let anchor_idx = cluster_members
                    .iter()
                    .copied()
                    .max_by(|a, b| {
                        node_degree[*a]
                            .cmp(&node_degree[*b])
                            .then_with(|| labels[*a].cmp(&labels[*b]))
                    })
                    .unwrap_or(cluster_members[0]);
                let anchor_label = labels[anchor_idx].clone();
                let layer = layer_info.layers[cluster_id];
                let rank_in_layer = by_layer[layer]
                    .iter()
                    .position(|candidate| *candidate == cluster_id)
                    .unwrap_or(0);
                let layer_count = by_layer[layer].len().max(1);
                let x_ratio = if layer_info.max_layer == 0 {
                    let total = by_layer[0].len().max(1);
                    if total == 1 {
                        0.5
                    } else {
                        0.16 + (rank_in_layer as f64 / (total - 1) as f64) * 0.68
                    }
                } else {
                    0.14 + (layer as f64 / layer_info.max_layer as f64) * 0.72
                };
                let y_ratio = if layer_count == 1 {
                    0.5
                } else {
                    0.16 + (rank_in_layer as f64 / (layer_count - 1) as f64) * 0.68
                };
                let name = choose_cluster_name(&cluster_members, labels, internal_nodes, config);
                let kind = infer_cluster_kind(&name, &cluster_members, internal_nodes, config);
                let internal_member_count = cluster_members
                    .iter()
                    .filter(|member| internal_nodes.is_none_or(|set| set.contains(member)))
                    .count();
                let external_member_count =
                    cluster_members.len().saturating_sub(internal_member_count);

                ClusterNode {
                    id: cluster_id,
                    name,
                    kind,
                    members: cluster_members,
                    internal_member_count,
                    external_member_count,
                    anchor_label,
                    layer,
                    x_ratio,
                    y_ratio,
                    inbound_weight: inbound_weight[cluster_id],
                    outbound_weight: outbound_weight[cluster_id],
                    internal_weight: internal_weight[cluster_id],
                }
            })
            .collect::<Vec<_>>();
        if config.disambiguate_duplicate_names() {
            uniquify_cluster_names(&mut clusters);
        }

        let should_default_to_overview = labels.len() >= OVERVIEW_NODE_THRESHOLD
            && clusters.len() >= MIN_OVERVIEW_CLUSTERS
            && clusters.len() + 6 < labels.len();

        Some(Self {
            clusters,
            edges,
            cluster_of_node,
            max_layer: layer_info.max_layer,
            should_default_to_overview,
        })
    }
}

#[derive(Debug, Clone)]
struct LayerInfo {
    layers: Vec<usize>,
    max_layer: usize,
}

#[derive(Debug, Clone)]
struct SemanticAssignments {
    cluster_of_node: Vec<usize>,
    cluster_names: Vec<String>,
}

fn build_undirected_adjacency(
    node_count: usize,
    edges: &[(usize, usize)],
    edge_weights: &[u32],
) -> Vec<Vec<(usize, u32)>> {
    let mut merged = vec![HashMap::<usize, u32>::new(); node_count];
    let weights = normalized_weights(edges.len(), edge_weights);
    for (edge_idx, &(from, to)) in edges.iter().enumerate() {
        if from >= node_count || to >= node_count || from == to {
            continue;
        }
        let weight = weights[edge_idx];
        *merged[from].entry(to).or_insert(0) += weight;
        *merged[to].entry(from).or_insert(0) += weight;
    }
    merged
        .into_iter()
        .map(|neighbors| neighbors.into_iter().collect())
        .collect()
}

fn label_propagation(adjacency: &[Vec<(usize, u32)>]) -> Vec<usize> {
    let mut labels = (0..adjacency.len()).collect::<Vec<_>>();
    let mut order = (0..adjacency.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| {
        adjacency[b]
            .len()
            .cmp(&adjacency[a].len())
            .then_with(|| a.cmp(&b))
    });

    for _ in 0..12 {
        let mut changed = false;
        for &node_idx in &order {
            if adjacency[node_idx].is_empty() {
                continue;
            }

            let mut scores = HashMap::<usize, u32>::new();
            for &(neighbor, weight) in &adjacency[node_idx] {
                *scores.entry(labels[neighbor]).or_insert(0) += weight.max(1);
            }

            let current = labels[node_idx];
            let best = scores
                .into_iter()
                .max_by(|(label_a, score_a), (label_b, score_b)| {
                    score_a
                        .cmp(score_b)
                        .then_with(|| (*label_a == current).cmp(&(*label_b == current)))
                        .then_with(|| label_b.cmp(label_a))
                })
                .map(|(label, _)| label)
                .unwrap_or(current);

            if best != current {
                labels[node_idx] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    labels
}

fn split_dominant_semantic_clusters(
    cluster_of_node: &mut [usize],
    semantic: &SemanticAssignments,
    adjacency: &[Vec<(usize, u32)>],
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
    total_nodes: usize,
    config: &ClusteringConfig,
) {
    let mut members = HashMap::<usize, Vec<usize>>::new();
    for (node_idx, &cluster_id) in cluster_of_node.iter().enumerate() {
        members.entry(cluster_id).or_default().push(node_idx);
    }

    let mut next_cluster_id = cluster_of_node.iter().copied().max().unwrap_or(0) + 1;
    for (cluster_id, cluster_members) in members {
        let semantic_name = semantic
            .cluster_names
            .get(cluster_id)
            .map(String::as_str)
            .unwrap_or(ROOT_NAMESPACE);
        let cluster_share = cluster_members.len() as f64 / total_nodes.max(1) as f64;
        let should_split = !matches!(
            config.family_split_mode(semantic_name),
            crate::config::FamilySplitMode::Never
        ) && (semantic_name == ROOT_NAMESPACE
            || (cluster_members.len() >= config.effective_split_threshold()
                && cluster_share >= config.effective_max_cluster_share()
                && semantic_name != DEPS_NAMESPACE));
        if !should_split || cluster_members.len() < config.effective_split_threshold() {
            continue;
        }

        let local_index = cluster_members
            .iter()
            .enumerate()
            .map(|(idx, node_id)| (*node_id, idx))
            .collect::<HashMap<_, _>>();
        let mut local_adjacency = vec![Vec::<(usize, u32)>::new(); cluster_members.len()];
        for &node_id in &cluster_members {
            let Some(&local_node) = local_index.get(&node_id) else {
                continue;
            };
            for &(neighbor, weight) in &adjacency[node_id] {
                if let Some(&local_neighbor) = local_index.get(&neighbor) {
                    local_adjacency[local_node].push((local_neighbor, weight));
                }
            }
        }

        let local_clusters = label_propagation(&local_adjacency);
        let mut local_members = HashMap::<usize, Vec<usize>>::new();
        for (offset, &local_cluster) in local_clusters.iter().enumerate() {
            local_members
                .entry(local_cluster)
                .or_default()
                .push(cluster_members[offset]);
        }
        if local_members.len() < 2 {
            continue;
        }

        let meaningful_groups = local_members
            .values()
            .filter(|members| members.len() >= config.effective_min_cluster_size().max(2))
            .count();
        if meaningful_groups < 2 {
            continue;
        }

        let mut local_groups = local_members.into_values().collect::<Vec<_>>();
        local_groups.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
        if config.preserve_family_purity()
            && semantic_name != ROOT_NAMESPACE
            && !has_meaningful_family_split(
                &local_groups,
                labels,
                internal_nodes,
                config.effective_min_cluster_size().max(2),
                config,
            )
        {
            continue;
        }
        for (group_idx, group) in local_groups.into_iter().enumerate() {
            let assigned_cluster = if group_idx == 0 {
                cluster_id
            } else {
                let new_id = next_cluster_id;
                next_cluster_id += 1;
                new_id
            };
            for node_id in group {
                cluster_of_node[node_id] = assigned_cluster;
            }
        }
    }
}

fn split_large_generic_clusters(
    cluster_of_node: &mut [usize],
    labels: &[String],
    edges: &[(usize, usize)],
    edge_weights: &[u32],
    internal_nodes: Option<&HashSet<usize>>,
    config: &ClusteringConfig,
) {
    let mut members = HashMap::<usize, Vec<usize>>::new();
    for (node_idx, &cluster_id) in cluster_of_node.iter().enumerate() {
        members.entry(cluster_id).or_default().push(node_idx);
    }

    let total_nodes = labels.len().max(1);
    let mut next_cluster_id = cluster_of_node.iter().copied().max().unwrap_or(0) + 1;
    for (cluster_id, cluster_members) in members {
        if cluster_members.len() < config.effective_split_threshold() {
            continue;
        }

        let cluster_name =
            choose_cluster_base_name(&cluster_members, labels, internal_nodes, config);
        let cluster_share = cluster_members.len() as f64 / total_nodes as f64;
        if cluster_name != config.effective_fallback_family()
            || cluster_share < config.effective_max_cluster_share()
        {
            continue;
        }

        let local_index = cluster_members
            .iter()
            .enumerate()
            .map(|(idx, node_id)| (*node_id, idx))
            .collect::<HashMap<_, _>>();
        let weights = normalized_weights(edges.len(), edge_weights);
        let mut local_adjacency = vec![Vec::<(usize, u32)>::new(); cluster_members.len()];
        for (edge_idx, &(from, to)) in edges.iter().enumerate() {
            let Some(&local_from) = local_index.get(&from) else {
                continue;
            };
            let Some(&local_to) = local_index.get(&to) else {
                continue;
            };
            if local_from == local_to {
                continue;
            }
            let weight = weights[edge_idx];
            local_adjacency[local_from].push((local_to, weight));
            local_adjacency[local_to].push((local_from, weight));
        }

        let local_clusters = label_propagation(&local_adjacency);
        let mut local_members = HashMap::<usize, Vec<usize>>::new();
        for (offset, &local_cluster) in local_clusters.iter().enumerate() {
            local_members
                .entry(local_cluster)
                .or_default()
                .push(cluster_members[offset]);
        }

        let meaningful_groups = local_members
            .values()
            .filter(|members| members.len() >= config.effective_min_cluster_size().max(2))
            .count();
        if meaningful_groups < 2 {
            continue;
        }

        let mut local_groups = local_members.into_values().collect::<Vec<_>>();
        local_groups.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
        if config.preserve_family_purity()
            && !has_meaningful_family_split(
                &local_groups,
                labels,
                internal_nodes,
                config.effective_min_cluster_size().max(2),
                config,
            )
        {
            continue;
        }
        for (group_idx, group) in local_groups.into_iter().enumerate() {
            let assigned_cluster = if group_idx == 0 {
                cluster_id
            } else {
                let new_id = next_cluster_id;
                next_cluster_id += 1;
                new_id
            };
            for node_id in group {
                cluster_of_node[node_id] = assigned_cluster;
            }
        }
    }
}

fn merge_small_clusters(
    cluster_of_node: &mut [usize],
    labels: &[String],
    edges: &[(usize, usize)],
    edge_weights: &[u32],
    internal_nodes: Option<&HashSet<usize>>,
    config: &ClusteringConfig,
    min_cluster_size: usize,
) {
    let node_count = labels.len();
    let mut members = HashMap::<usize, Vec<usize>>::new();
    for (node_idx, &cluster_id) in cluster_of_node.iter().enumerate() {
        members.entry(cluster_id).or_default().push(node_idx);
    }

    let weights = normalized_weights(edges.len(), edge_weights);
    let family_by_cluster = members
        .iter()
        .map(|(cluster_id, cluster_members)| {
            (
                *cluster_id,
                choose_cluster_base_name(cluster_members, labels, internal_nodes, config),
            )
        })
        .collect::<HashMap<_, _>>();
    for (cluster_id, cluster_members) in members {
        if cluster_members.len() >= min_cluster_size
            || members_len(cluster_of_node, cluster_id) == node_count
        {
            continue;
        }

        let cluster_set = cluster_members.iter().copied().collect::<HashSet<_>>();
        let cluster_family = family_by_cluster
            .get(&cluster_id)
            .cloned()
            .unwrap_or_else(|| ROOT_NAMESPACE.to_string());
        let mut neighbor_scores = HashMap::<usize, u32>::new();
        for (edge_idx, &(from, to)) in edges.iter().enumerate() {
            let weight = weights[edge_idx];
            if cluster_set.contains(&from) && !cluster_set.contains(&to) {
                *neighbor_scores.entry(cluster_of_node[to]).or_insert(0) += weight;
            }
            if cluster_set.contains(&to) && !cluster_set.contains(&from) {
                *neighbor_scores.entry(cluster_of_node[from]).or_insert(0) += weight;
            }
        }

        if let Some((&best_cluster, _)) = neighbor_scores.iter().max_by(|a, b| {
            let same_family_a = family_by_cluster.get(a.0).is_some_and(|family| {
                family == &cluster_family && config.family_prefers_small_merge(&cluster_family)
            });
            let same_family_b = family_by_cluster.get(b.0).is_some_and(|family| {
                family == &cluster_family && config.family_prefers_small_merge(&cluster_family)
            });
            same_family_a
                .cmp(&same_family_b)
                .then_with(|| a.1.cmp(b.1))
                .then_with(|| b.0.cmp(a.0))
        }) {
            for node_idx in cluster_members {
                cluster_of_node[node_idx] = best_cluster;
            }
        }
    }
}

fn members_len(cluster_of_node: &[usize], cluster_id: usize) -> usize {
    cluster_of_node
        .iter()
        .filter(|candidate| **candidate == cluster_id)
        .count()
}

fn normalized_weights(edge_len: usize, edge_weights: &[u32]) -> Vec<u32> {
    if edge_weights.len() == edge_len {
        edge_weights.iter().map(|weight| (*weight).max(1)).collect()
    } else {
        vec![1; edge_len]
    }
}

fn apply_must_group_constraints(
    cluster_of_node: &mut [usize],
    labels: &[String],
    config: &ClusteringConfig,
) {
    for constraint in config.constraints_of_type(ClusteringConstraintType::MustGroup) {
        let matched_nodes = labels
            .iter()
            .enumerate()
            .filter(|(_, label)| constraint.matches_members(label))
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        if matched_nodes.len() < 2 {
            continue;
        }

        let mut cluster_counts = HashMap::<usize, usize>::new();
        for &node_idx in &matched_nodes {
            *cluster_counts.entry(cluster_of_node[node_idx]).or_insert(0) += 1;
        }
        let target_cluster = cluster_counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|(cluster_id, _)| cluster_id)
            .unwrap_or(cluster_of_node[matched_nodes[0]]);

        for node_idx in matched_nodes {
            cluster_of_node[node_idx] = target_cluster;
        }
    }
}

fn apply_must_separate_constraints(
    cluster_of_node: &mut [usize],
    labels: &[String],
    config: &ClusteringConfig,
) {
    let mut next_cluster_id = cluster_of_node.iter().copied().max().unwrap_or(0) + 1;

    for constraint in config.constraints_of_type(ClusteringConstraintType::MustSeparate) {
        let left_nodes = labels
            .iter()
            .enumerate()
            .filter(|(_, label)| constraint.matches_left(label))
            .map(|(idx, _)| idx)
            .collect::<HashSet<_>>();
        let right_nodes = labels
            .iter()
            .enumerate()
            .filter(|(_, label)| constraint.matches_right(label))
            .map(|(idx, _)| idx)
            .collect::<HashSet<_>>();

        if left_nodes.is_empty() || right_nodes.is_empty() {
            continue;
        }

        let conflicting_clusters = left_nodes
            .iter()
            .filter_map(|node_idx| {
                let cluster_id = cluster_of_node[*node_idx];
                right_nodes
                    .iter()
                    .any(|right_idx| cluster_of_node[*right_idx] == cluster_id)
                    .then_some(cluster_id)
            })
            .collect::<HashSet<_>>();

        for conflicting_cluster in conflicting_clusters {
            let mut right_exclusive = right_nodes
                .iter()
                .copied()
                .filter(|node_idx| {
                    cluster_of_node[*node_idx] == conflicting_cluster
                        && !left_nodes.contains(node_idx)
                })
                .collect::<Vec<_>>();
            let mut left_exclusive = left_nodes
                .iter()
                .copied()
                .filter(|node_idx| {
                    cluster_of_node[*node_idx] == conflicting_cluster
                        && !right_nodes.contains(node_idx)
                })
                .collect::<Vec<_>>();

            let nodes_to_move = if !right_exclusive.is_empty() {
                &mut right_exclusive
            } else if !left_exclusive.is_empty() {
                &mut left_exclusive
            } else {
                continue;
            };

            let new_cluster = next_cluster_id;
            next_cluster_id += 1;
            for node_idx in nodes_to_move.iter().copied() {
                cluster_of_node[node_idx] = new_cluster;
            }
        }
    }
}

fn namespace_assignments(
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
    config: &ClusteringConfig,
) -> Option<SemanticAssignments> {
    let root_token_counts = repeated_root_token_counts(labels, internal_nodes);
    let known_path_heads = known_internal_path_heads(labels, internal_nodes);
    let mut namespaces = Vec::with_capacity(labels.len());
    let mut id_by_namespace = HashMap::<String, usize>::new();
    let mut cluster_names = Vec::<String>::new();

    for (idx, label) in labels.iter().enumerate() {
        let is_internal = internal_nodes.is_none_or(|set| set.contains(&idx));
        let namespace = namespace_key(
            label,
            is_internal,
            &root_token_counts,
            &known_path_heads,
            config,
        )?;
        let next_id = id_by_namespace.len();
        let cluster_id = *id_by_namespace.entry(namespace.clone()).or_insert(next_id);
        if cluster_id == cluster_names.len() {
            cluster_names.push(namespace);
        }
        namespaces.push(cluster_id);
    }

    let unique = namespaces.iter().copied().collect::<HashSet<_>>().len();
    if unique < 2 {
        None
    } else {
        Some(SemanticAssignments {
            cluster_of_node: namespaces,
            cluster_names,
        })
    }
}

fn namespace_key(
    label: &str,
    is_internal: bool,
    root_token_counts: &HashMap<String, usize>,
    known_path_heads: &HashSet<String>,
    config: &ClusteringConfig,
) -> Option<String> {
    if let Some(name) = config.matching_family_name(label) {
        return Some(name);
    }

    if !is_internal {
        return if config.effective_collapse_external() {
            Some(DEPS_NAMESPACE.to_string())
        } else {
            Some(label.to_string())
        };
    }

    if let Some((head, _)) = label.split_once('/') {
        return Some(head.to_string());
    }

    if config.include_exact_roots_for_known_heads() && known_path_heads.contains(label) {
        return Some(label.to_string());
    }

    if matches!(label, "cli" | "ext" | "libs" | "runtime") {
        return Some(label.to_string());
    }

    for (prefix, namespace) in [
        ("cli_", "cli"),
        ("ext_", "ext"),
        ("libs_", "libs"),
        ("runtime_", "runtime"),
        ("deno_", "deno"),
    ] {
        if label.starts_with(prefix) {
            return Some(namespace.to_string());
        }
    }

    if let Some(token) = root_token(label)
        && root_token_counts.get(token).copied().unwrap_or(0)
            >= config.effective_root_token_min_repeats()
    {
        return Some(token.to_string());
    }

    if label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Some(ROOT_NAMESPACE.to_string())
    } else {
        None
    }
}

fn repeated_root_token_counts(
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
) -> HashMap<String, usize> {
    let mut counts = HashMap::<String, usize>::new();
    for (idx, label) in labels.iter().enumerate() {
        let is_internal = internal_nodes.is_none_or(|set| set.contains(&idx));
        if !is_internal || label.contains('/') {
            continue;
        }
        if let Some(token) = root_token(label) {
            *counts.entry(token.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

fn known_internal_path_heads(
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
) -> HashSet<String> {
    let mut heads = HashSet::<String>::new();
    for (idx, label) in labels.iter().enumerate() {
        let is_internal = internal_nodes.is_none_or(|set| set.contains(&idx));
        if !is_internal {
            continue;
        }
        if let Some((head, _)) = label.split_once('/') {
            heads.insert(head.to_string());
        }
    }
    heads
}

fn root_token(label: &str) -> Option<&str> {
    for separator in ['_', '-'] {
        if let Some((prefix, _)) = label.split_once(separator)
            && prefix.len() >= 3
        {
            return Some(prefix);
        }
    }
    None
}

fn choose_cluster_base_name(
    members: &[usize],
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
    config: &ClusteringConfig,
) -> String {
    let mut counts = HashMap::<String, usize>::new();
    let root_token_counts = repeated_root_token_counts(labels, internal_nodes);
    let known_path_heads = known_internal_path_heads(labels, internal_nodes);
    for &member in members {
        let is_internal = internal_nodes.is_none_or(|set| set.contains(&member));
        if let Some(namespace) = namespace_key(
            &labels[member],
            is_internal,
            &root_token_counts,
            &known_path_heads,
            config,
        ) {
            *counts.entry(namespace).or_insert(0) += 1;
        }
    }
    let best = counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(name, _)| name)
        .unwrap_or_else(|| labels[members[0]].clone());

    if best == ROOT_NAMESPACE {
        let mut root_prefixes = HashMap::<String, usize>::new();
        let mut root_like_members = 0usize;
        for &member in members {
            let label = &labels[member];
            if let Some(prefix) = root_token(label) {
                root_like_members += 1;
                *root_prefixes.entry(prefix.to_string()).or_insert(0) += 1;
            }
        }
        let dominant = root_prefixes
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .filter(|(_, count)| *count >= 2 && *count * 2 >= root_like_members.max(1))
            .map(|(name, _)| name);
        dominant.unwrap_or_else(|| config.effective_fallback_family().to_string())
    } else if best == DEPS_NAMESPACE {
        "deps".to_string()
    } else {
        best
    }
}

fn resolve_cluster_name(name: &str, config: &ClusteringConfig) -> String {
    config.display_name_for(name)
}

fn choose_cluster_name(
    members: &[usize],
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
    config: &ClusteringConfig,
) -> String {
    let base_name = choose_cluster_base_name(members, labels, internal_nodes, config);
    resolve_cluster_name(&base_name, config)
}

fn has_meaningful_family_split(
    groups: &[Vec<usize>],
    labels: &[String],
    internal_nodes: Option<&HashSet<usize>>,
    min_cluster_size: usize,
    config: &ClusteringConfig,
) -> bool {
    groups
        .iter()
        .filter(|group| group.len() >= min_cluster_size)
        .map(|group| choose_cluster_base_name(group, labels, internal_nodes, config))
        .collect::<HashSet<_>>()
        .len()
        >= 2
}

fn uniquify_cluster_names(clusters: &mut [ClusterNode]) {
    let mut indices_by_name = HashMap::<String, Vec<usize>>::new();
    for (idx, cluster) in clusters.iter().enumerate() {
        indices_by_name
            .entry(cluster.name.clone())
            .or_default()
            .push(idx);
    }

    for (base_name, indices) in indices_by_name {
        if indices.len() < 2 {
            continue;
        }

        let mut taken = HashSet::<String>::new();
        let mut pending_names = HashMap::<usize, String>::new();
        for &cluster_idx in &indices {
            let Some(suffix) = cluster_name_suffix(&base_name, &clusters[cluster_idx].anchor_label)
            else {
                continue;
            };
            let candidate = format!("{base_name} [{suffix}]");
            if taken.insert(candidate.clone()) {
                pending_names.insert(cluster_idx, candidate);
            }
        }

        let mut ordinal = 1usize;
        for &cluster_idx in &indices {
            if let Some(name) = pending_names.remove(&cluster_idx) {
                clusters[cluster_idx].name = name;
                continue;
            }

            loop {
                let candidate = format!("{base_name} [{ordinal}]");
                ordinal += 1;
                if taken.insert(candidate.clone()) {
                    clusters[cluster_idx].name = candidate;
                    break;
                }
            }
        }
    }
}

fn cluster_name_suffix(base_name: &str, anchor_label: &str) -> Option<String> {
    let trimmed = anchor_label.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(base_name) {
        return None;
    }

    for separator in ['/', '_', '-'] {
        let prefix = format!("{base_name}{separator}");
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            return compact_label(rest, 16);
        }
    }

    if let Some((_, tail)) = trimmed.rsplit_once('/') {
        return compact_label(tail, 16);
    }
    if let Some((_, tail)) = trimmed.rsplit_once('_') {
        return compact_label(tail, 16);
    }
    if let Some((_, tail)) = trimmed.rsplit_once('-') {
        return compact_label(tail, 16);
    }

    compact_label(trimmed, 16)
}

fn compact_label(input: &str, max_chars: usize) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut shortened = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        shortened.push(ch);
    }

    if shortened.is_empty() {
        None
    } else {
        Some(shortened)
    }
}

fn cluster_kind_from_hint(hint: &ClusterKindHint) -> ClusterKind {
    match hint {
        ClusterKindHint::Workspace => ClusterKind::Workspace,
        ClusterKindHint::Deps => ClusterKind::Deps,
        ClusterKindHint::Entry => ClusterKind::Entry,
        ClusterKindHint::External => ClusterKind::External,
        ClusterKindHint::Infra => ClusterKind::Infra,
        ClusterKindHint::Domain => ClusterKind::Domain,
        ClusterKindHint::Group => ClusterKind::Group,
    }
}

fn infer_cluster_kind(
    name: &str,
    members: &[usize],
    internal_nodes: Option<&HashSet<usize>>,
    config: &ClusteringConfig,
) -> ClusterKind {
    if let Some(hint) = config.kind_hint_for(name) {
        return cluster_kind_from_hint(hint);
    }

    let internal_count = members
        .iter()
        .filter(|member| internal_nodes.is_none_or(|set| set.contains(member)))
        .count();

    if internal_count == 0 {
        return ClusterKind::External;
    }

    match config.effective_kind_mode() {
        ClusterKindMode::ExplicitOnly => ClusterKind::Group,
        ClusterKindMode::ExplicitThenHeuristic => {
            let lower = name.to_lowercase();
            if lower == "deps" {
                ClusterKind::Deps
            } else if lower == "workspace" {
                ClusterKind::Workspace
            } else if matches!(
                lower.as_str(),
                "app" | "apps" | "website" | "web" | "frontend" | "cmd" | "service"
            ) {
                ClusterKind::Entry
            } else if matches!(
                lower.as_str(),
                "runtime" | "infra" | "platform" | "shared" | "common"
            ) {
                ClusterKind::Infra
            } else {
                ClusterKind::Group
            }
        }
    }
}

fn compute_cluster_layers(cluster_count: usize, edges: &[ClusterEdge]) -> LayerInfo {
    let mut graph = DiGraph::<(), ()>::new();
    let node_indices = (0..cluster_count)
        .map(|_| graph.add_node(()))
        .collect::<Vec<_>>();
    for edge in edges {
        if edge.from != edge.to {
            graph.add_edge(node_indices[edge.from], node_indices[edge.to], ());
        }
    }

    let sccs = kosaraju_scc(&graph);
    let mut component_of = vec![0usize; cluster_count];
    for (component_idx, component) in sccs.iter().enumerate() {
        for node in component {
            component_of[node.index()] = component_idx;
        }
    }

    let mut successors = vec![HashSet::<usize>::new(); sccs.len()];
    let mut indegree = vec![0usize; sccs.len()];
    for edge in edges {
        let from_component = component_of[edge.from];
        let to_component = component_of[edge.to];
        if from_component != to_component && successors[from_component].insert(to_component) {
            indegree[to_component] += 1;
        }
    }

    let mut queue = VecDeque::new();
    for (component_idx, &degree) in indegree.iter().enumerate() {
        if degree == 0 {
            queue.push_back(component_idx);
        }
    }

    let mut component_layer = vec![0usize; sccs.len()];
    let mut indegree_work = indegree.clone();
    while let Some(component_idx) = queue.pop_front() {
        let next_layer = component_layer[component_idx] + 1;
        for &next in &successors[component_idx] {
            component_layer[next] = component_layer[next].max(next_layer);
            indegree_work[next] = indegree_work[next].saturating_sub(1);
            if indegree_work[next] == 0 {
                queue.push_back(next);
            }
        }
    }

    let layers = component_of
        .iter()
        .map(|component_idx| component_layer[*component_idx])
        .collect::<Vec<_>>();
    let max_layer = layers.iter().copied().max().unwrap_or(0);
    LayerInfo { layers, max_layer }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_namespace_grouping_for_path_labels() {
        let labels = vec![
            "cli/args".to_string(),
            "cli/tools".to_string(),
            "ext/http".to_string(),
            "ext/node".to_string(),
            "runtime/js".to_string(),
            "runtime/ops".to_string(),
        ];
        let edges = vec![(0, 2), (1, 2), (4, 2), (5, 3)];
        let weights = vec![1; edges.len()];
        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            None,
            &ClusteringConfig::default(),
        )
        .expect("map");
        assert!(map.clusters.iter().any(|cluster| cluster.name == "cli"));
        assert!(map.clusters.iter().any(|cluster| cluster.name == "ext"));
        assert!(map.clusters.iter().any(|cluster| cluster.name == "runtime"));
    }

    #[test]
    fn groups_external_nodes_into_deps_cluster() {
        let labels = vec![
            "cli/args".to_string(),
            "cli/tools".to_string(),
            "serde".to_string(),
            "tokio".to_string(),
        ];
        let edges = vec![(0, 2), (1, 2), (1, 3)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize]);
        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
        )
        .expect("map");
        let deps = map
            .clusters
            .iter()
            .find(|cluster| cluster.name == "deps")
            .expect("deps cluster");
        assert_eq!(deps.internal_member_count, 0);
        assert_eq!(deps.external_member_count, 2);
        assert!(deps.is_dependency_sink());
        assert_eq!(deps.overview_role(), ClusterOverviewRole::ExternalSink);
    }

    #[test]
    fn groups_workspace_root_crates_under_deno() {
        let labels = vec![
            "deno_core".to_string(),
            "deno_ast".to_string(),
            "import_map".to_string(),
            "node_resolver".to_string(),
            "cli/tools".to_string(),
            "cli/lsp".to_string(),
            "ext/http".to_string(),
            "ext/node".to_string(),
            "anyhow".to_string(),
            "serde".to_string(),
        ];
        let edges = vec![
            (0, 8),
            (1, 9),
            (2, 8),
            (3, 9),
            (4, 0),
            (5, 0),
            (6, 0),
            (7, 1),
        ];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([
            0usize, 1usize, 2usize, 3usize, 4usize, 5usize, 6usize, 7usize,
        ]);
        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
        )
        .expect("map");
        assert!(map.clusters.iter().any(|cluster| cluster.name == "deno"));
        assert!(map.clusters.iter().any(|cluster| cluster.name == "cli"));
        assert!(map.clusters.iter().any(|cluster| cluster.name == "deps"));
    }

    #[test]
    fn exact_root_labels_join_matching_namespaced_family() {
        let labels = vec![
            "serde_v8".to_string(),
            "serde_v8/ser".to_string(),
            "serde_v8/de".to_string(),
            "ops".to_string(),
            "ops/fast".to_string(),
            "cli/tools".to_string(),
            "ext/node".to_string(),
        ];
        let edges = vec![(5, 0), (5, 3), (6, 1), (6, 4), (1, 2), (3, 4)];
        let weights = vec![2, 2, 1, 1, 1, 1];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize, 4usize, 5usize, 6usize]);
        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
        )
        .expect("map");

        let serde_cluster = map
            .clusters
            .iter()
            .find(|cluster| cluster.name.starts_with("serde_v8"))
            .expect("serde_v8 cluster");
        assert!(serde_cluster.members.contains(&0));
        assert!(serde_cluster.members.contains(&1));
        assert!(serde_cluster.members.contains(&2));

        let ops_cluster = map
            .clusters
            .iter()
            .find(|cluster| cluster.name.starts_with("ops"))
            .expect("ops cluster");
        assert!(ops_cluster.members.contains(&3));
        assert!(ops_cluster.members.contains(&4));
        assert!(ops_cluster.members.len() >= 2);
    }

    #[test]
    fn groups_repeated_root_tokens_without_falling_back_to_global_propagation() {
        let labels = vec![
            "node_resolver".to_string(),
            "node_cache".to_string(),
            "import_map".to_string(),
            "import_resolver".to_string(),
            "cli/tools".to_string(),
        ];
        let edges = vec![(4, 0), (4, 2), (0, 1), (2, 3)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize, 4usize]);
        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
        )
        .expect("map");
        assert!(map.clusters.iter().any(|cluster| cluster.name == "node"));
        assert!(map.clusters.iter().any(|cluster| cluster.name == "import"));
    }

    #[test]
    fn does_not_promote_singleton_prefixes_to_cluster_names() {
        let labels = vec![
            "commands".to_string(),
            "main".to_string(),
            "parser".to_string(),
            "tui".to_string(),
            "git_scanner".to_string(),
            "graph_builder".to_string(),
        ];
        let edges = vec![(0, 1), (1, 2), (2, 3), (3, 4), (3, 5)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize, 4usize, 5usize]);
        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
        )
        .expect("map");
        assert!(!map.clusters.iter().any(|cluster| cluster.name == "git"));
        assert!(!map.clusters.iter().any(|cluster| cluster.name == "graph"));
        assert!(map.clusters.len() >= 2);
    }

    #[test]
    fn applies_cluster_aliases_to_user_facing_names() {
        let labels = vec![
            "cli/args".to_string(),
            "cli/tools".to_string(),
            "serde".to_string(),
            "tokio".to_string(),
        ];
        let edges = vec![(0, 2), (1, 2), (1, 3)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize]);
        let mut config = ClusteringConfig::default();
        config
            .cluster_aliases
            .insert("deps".to_string(), "third_party".to_string());
        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");
        assert!(
            map.clusters
                .iter()
                .any(|cluster| cluster.name == "third_party")
        );
    }

    #[test]
    fn applies_cluster_kind_overrides_to_alias_name() {
        let labels = vec![
            "cli/args".to_string(),
            "cli/tools".to_string(),
            "serde".to_string(),
            "tokio".to_string(),
        ];
        let edges = vec![(0, 2), (1, 2), (1, 3)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize]);
        let mut config = ClusteringConfig::default();
        config
            .cluster_aliases
            .insert("deps".to_string(), "third_party".to_string());
        config
            .cluster_kinds
            .insert("third_party".to_string(), ClusterKindHint::Deps);
        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");
        assert!(
            map.clusters.iter().any(|cluster| {
                cluster.name == "third_party" && cluster.kind == ClusterKind::Deps
            })
        );
    }

    #[test]
    fn explicit_only_kind_mode_disables_name_heuristics() {
        let labels = vec![
            "runtime/core".to_string(),
            "runtime/ops".to_string(),
            "cli/tools".to_string(),
            "cli/lsp".to_string(),
        ];
        let edges = vec![(0, 1), (2, 3)];
        let weights = vec![3, 2];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize]);
        let mut config = ClusteringConfig::default();
        config.presentation = Some(crate::config::ClusteringPresentationConfig {
            aliases: HashMap::new(),
            kinds: HashMap::new(),
            kind_mode: crate::config::ClusterKindMode::ExplicitOnly,
            ..Default::default()
        });

        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");
        let runtime = map
            .clusters
            .iter()
            .find(|cluster| cluster.name == "runtime")
            .expect("runtime cluster");
        assert_eq!(runtime.kind, ClusterKind::Group);
    }

    #[test]
    fn explicit_only_kind_mode_still_respects_explicit_overrides() {
        let labels = vec![
            "runtime/core".to_string(),
            "runtime/ops".to_string(),
            "cli/tools".to_string(),
            "cli/lsp".to_string(),
        ];
        let edges = vec![(0, 1), (2, 3)];
        let weights = vec![3, 2];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize]);
        let mut config = ClusteringConfig::default();
        config.presentation = Some(crate::config::ClusteringPresentationConfig {
            aliases: HashMap::new(),
            kinds: HashMap::from([("runtime".to_string(), ClusterKindHint::Infra)]),
            kind_mode: crate::config::ClusterKindMode::ExplicitOnly,
            ..Default::default()
        });

        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");
        let runtime = map
            .clusters
            .iter()
            .find(|cluster| cluster.name == "runtime")
            .expect("runtime cluster");
        assert_eq!(runtime.kind, ClusterKind::Infra);
    }

    #[test]
    fn mixed_internal_cluster_stays_primary_architecture() {
        let cluster = ClusterNode {
            id: 0,
            name: "core".to_string(),
            kind: ClusterKind::Group,
            members: vec![0, 1, 2, 3],
            internal_member_count: 3,
            external_member_count: 1,
            anchor_label: "core/a".to_string(),
            layer: 0,
            x_ratio: 0.5,
            y_ratio: 0.5,
            inbound_weight: 12,
            outbound_weight: 8,
            internal_weight: 20,
        };

        assert!(cluster.is_internal_bearing());
        assert!(!cluster.is_dependency_sink());
        assert_eq!(
            cluster.overview_role(),
            ClusterOverviewRole::PrimaryArchitecture
        );
    }

    #[test]
    fn tiny_mostly_external_mixed_cluster_is_support_cluster() {
        let cluster = ClusterNode {
            id: 0,
            name: "sdk".to_string(),
            kind: ClusterKind::Group,
            members: vec![0, 1, 2, 3, 4],
            internal_member_count: 1,
            external_member_count: 4,
            anchor_label: "sdk".to_string(),
            layer: 0,
            x_ratio: 0.5,
            y_ratio: 0.5,
            inbound_weight: 10,
            outbound_weight: 14,
            internal_weight: 2,
        };

        assert_eq!(cluster.overview_role(), ClusterOverviewRole::SupportCluster);
        assert!(!cluster.is_dependency_sink());
    }

    #[test]
    fn duplicate_family_names_are_disambiguated_for_display() {
        let labels = vec![
            "std/fs".to_string(),
            "std/io".to_string(),
            "std/path".to_string(),
            "std/http".to_string(),
            "cli/a".to_string(),
            "cli/b".to_string(),
        ];
        let edges = vec![(0, 1), (2, 3), (4, 5)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize, 4usize, 5usize]);
        let config = ClusteringConfig {
            strategy: ClusteringStrategy::Structural,
            ..Default::default()
        };

        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");

        let std_names = map
            .clusters
            .iter()
            .filter(|cluster| cluster.name.starts_with("std"))
            .map(|cluster| cluster.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(std_names.len(), 2);
        assert_eq!(
            std_names.iter().collect::<HashSet<_>>().len(),
            std_names.len(),
            "duplicate semantic families should get unique display names",
        );
    }

    #[test]
    fn hybrid_clustering_keeps_semantically_pure_family_together() {
        let labels = vec![
            "std/fs".to_string(),
            "std/path".to_string(),
            "std/io".to_string(),
            "std/http".to_string(),
            "std/bytes".to_string(),
            "std/process".to_string(),
            "cli/tools".to_string(),
            "cli/lsp".to_string(),
        ];
        let edges = vec![(0, 1), (1, 2), (3, 4), (4, 5), (6, 7), (6, 0), (7, 3)];
        let weights = vec![5, 5, 5, 5, 3, 1, 1];
        let internal = HashSet::from([
            0usize, 1usize, 2usize, 3usize, 4usize, 5usize, 6usize, 7usize,
        ]);

        let map = ArchitectureMap::build(
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
        )
        .expect("map");

        let std_clusters = map
            .clusters
            .iter()
            .filter(|cluster| cluster.name.starts_with("std"))
            .collect::<Vec<_>>();
        assert_eq!(
            std_clusters.len(),
            1,
            "pure semantic families should not be split into multiple same-family clusters",
        );
    }

    #[test]
    fn merge_small_clusters_prefers_same_family_neighbor() {
        let labels = vec![
            "std/fs".to_string(),
            "std/io".to_string(),
            "std/path".to_string(),
            "cli/tools".to_string(),
            "cli/lsp".to_string(),
        ];
        let edges = vec![(0, 1), (0, 3), (3, 4)];
        let weights = vec![1, 5, 2];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize, 4usize]);
        let mut cluster_of_node = vec![0usize, 1usize, 1usize, 2usize, 2usize];

        merge_small_clusters(
            &mut cluster_of_node,
            &labels,
            &edges,
            &weights,
            Some(&internal),
            &ClusteringConfig::default(),
            2,
        );

        assert_eq!(
            cluster_of_node[0], cluster_of_node[1],
            "small semantic-family clusters should merge back into their family before stronger unrelated hubs"
        );
        assert_ne!(cluster_of_node[0], cluster_of_node[3]);
    }

    #[test]
    fn must_group_constraint_merges_selected_members() {
        let labels = vec![
            "core".to_string(),
            "runtime/ops".to_string(),
            "cli/tools".to_string(),
            "ext/node".to_string(),
        ];
        let edges = vec![(2, 0), (2, 1), (3, 1)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize]);
        let mut config = ClusteringConfig::default();
        config.strategy = ClusteringStrategy::Namespace;
        config.structural = Some(crate::config::ClusteringStructuralConfig {
            enabled: false,
            ..Default::default()
        });
        config.constraints = vec![
            crate::config::ClusteringConstraint::must_group(vec![
                "core".to_string(),
                "runtime/**".to_string(),
            ])
            .expect("constraint"),
        ];

        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");
        assert_eq!(map.cluster_of_node[0], map.cluster_of_node[1]);
    }

    #[test]
    fn must_separate_constraint_splits_conflicting_family_members() {
        let labels = vec![
            "core/a".to_string(),
            "core/b".to_string(),
            "core/c".to_string(),
            "cli/tools".to_string(),
        ];
        let edges = vec![(3, 0), (3, 1), (3, 2)];
        let weights = vec![1; edges.len()];
        let internal = HashSet::from([0usize, 1usize, 2usize, 3usize]);
        let mut config = ClusteringConfig::default();
        config.strategy = ClusteringStrategy::Namespace;
        config.structural = Some(crate::config::ClusteringStructuralConfig {
            enabled: false,
            ..Default::default()
        });
        config.constraints = vec![
            crate::config::ClusteringConstraint::must_separate(
                vec!["core/a".to_string()],
                vec!["core/b".to_string()],
            )
            .expect("constraint"),
        ];

        let map = ArchitectureMap::build(&labels, &edges, &weights, Some(&internal), &config)
            .expect("map");
        assert_ne!(map.cluster_of_node[0], map.cluster_of_node[1]);
    }
}
