//! Blast Radius Cartography — transitive impact analysis.
//!
//! Computes per-module blast radius (how much of the graph is affected by a change),
//! identifies articulation points (structural keystones), and finds critical
//! dependency chains (longest weighted paths).
//!
//! All operations are O(V+E) or O(V*(V+E)) and operate on the existing
//! `petgraph::DiGraph<String, u32>`.

use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::models::{
    ArticulationPoint, BlastRadiusReport, BlastRadiusSummary, CascadePath, ModuleImpact,
};

/// Computes the full blast radius report for a dependency graph.
///
/// Orchestrates articulation point detection, per-module blast radius
/// computation, and critical path finding.
pub fn compute_blast_radius_report(
    graph: &DiGraph<String, u32>,
    max_critical_paths: usize,
) -> BlastRadiusReport {
    let n = graph.node_count();
    if n == 0 {
        return BlastRadiusReport {
            impacts: Vec::new(),
            articulation_points: Vec::new(),
            critical_paths: Vec::new(),
            summary: BlastRadiusSummary {
                articulation_point_count: 0,
                max_blast_score: 0.0,
                most_impactful_module: String::new(),
                mean_blast_score: 0.0,
                longest_chain_depth: 0,
            },
        };
    }

    // 1. Articulation points
    let ap_list = find_articulation_points(graph);
    let ap_name_set: HashSet<&str> = ap_list.iter().map(|a| a.module_name.as_str()).collect();
    let ap_node_set: HashSet<NodeIndex> = graph
        .node_indices()
        .filter(|&ni| ap_name_set.contains(graph[ni].as_str()))
        .collect();

    // 2. Blast radius per module
    let impacts = compute_blast_radius(graph, &ap_node_set);

    // 3. Critical paths
    let critical_paths = find_critical_paths(graph, max_critical_paths);

    // 4. Summary
    let max_blast_score = impacts.first().map(|m| m.blast_score).unwrap_or(0.0);
    let most_impactful = impacts
        .first()
        .map(|m| m.module_name.clone())
        .unwrap_or_default();
    let mean_blast_score = if !impacts.is_empty() {
        impacts.iter().map(|m| m.blast_score).sum::<f64>() / impacts.len() as f64
    } else {
        0.0
    };
    let longest_chain_depth = critical_paths.iter().map(|p| p.depth).max().unwrap_or(0);

    BlastRadiusReport {
        impacts,
        articulation_points: ap_list,
        critical_paths,
        summary: BlastRadiusSummary {
            articulation_point_count: ap_node_set.len(),
            max_blast_score,
            most_impactful_module: most_impactful,
            mean_blast_score,
            longest_chain_depth,
        },
    }
}

/// Computes the transitive blast radius for every module.
///
/// For each module M, performs a weighted BFS on the reverse dependency graph
/// (following incoming edges) to find all modules that transitively depend on M.
///
/// In this graph, edge A→B means "A imports B", so a change in B propagates
/// to A. We follow **incoming** edges from B to find affected modules.
///
/// Impact at distance d is: `ln(1 + edge_weight) / (1 + d)²`
fn compute_blast_radius(
    graph: &DiGraph<String, u32>,
    articulation_set: &HashSet<NodeIndex>,
) -> Vec<ModuleImpact> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }

    let mut impacts = Vec::with_capacity(n);

    for source in graph.node_indices() {
        let (downstream_count, weighted_reach) = bfs_blast(graph, source);

        let blast_score = if n > 1 {
            (weighted_reach / (n - 1) as f64).min(1.0)
        } else {
            0.0
        };

        impacts.push(ModuleImpact {
            module_name: graph[source].clone(),
            blast_score,
            downstream_count,
            weighted_reach,
            is_articulation_point: articulation_set.contains(&source),
        });
    }

    impacts.sort_by(|a, b| {
        b.blast_score
            .partial_cmp(&a.blast_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    impacts
}

/// BFS from `source` following incoming edges (reverse direction).
///
/// Returns (downstream_count, weighted_reach).
fn bfs_blast(graph: &DiGraph<String, u32>, source: NodeIndex) -> (usize, f64) {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(source);
    queue.push_back((source, 0u32));

    let mut downstream_count = 0usize;
    let mut weighted_reach = 0.0f64;

    while let Some((current, dist)) = queue.pop_front() {
        // Follow incoming edges: who depends on `current`?
        for edge in graph.edges_directed(current, petgraph::Direction::Incoming) {
            let neighbor = edge.source();
            if visited.insert(neighbor) {
                let new_dist = dist + 1;
                let edge_weight = *edge.weight();
                // Inverse-square decay * log-dampened coupling intensity
                let impact = (1.0 + edge_weight as f64).ln() / (1.0 + new_dist as f64).powi(2);
                weighted_reach += impact;
                downstream_count += 1;
                queue.push_back((neighbor, new_dist));
            }
        }
    }

    (downstream_count, weighted_reach)
}

/// Computes the blast radius for a single selected node (for TUI overlay).
///
/// Returns `Vec<(affected_node, distance, decay_weighted_impact)>` for rendering.
pub fn compute_single_node_blast(
    graph: &DiGraph<String, u32>,
    source: NodeIndex,
) -> Vec<(NodeIndex, u32, f64)> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(source);
    queue.push_back((source, 0u32));

    while let Some((current, dist)) = queue.pop_front() {
        for edge in graph.edges_directed(current, petgraph::Direction::Incoming) {
            let neighbor = edge.source();
            if visited.insert(neighbor) {
                let new_dist = dist + 1;
                let w = *edge.weight();
                let impact = (1.0 + w as f64).ln() / (1.0 + new_dist as f64).powi(2);
                result.push((neighbor, new_dist, impact));
                queue.push_back((neighbor, new_dist));
            }
        }
    }

    result
}

// ── Articulation Point Detection ──

/// Finds all articulation points (cut vertices) in the graph.
///
/// Uses iterative Tarjan's algorithm on the undirected view of the graph.
/// An articulation point is a node whose removal disconnects the graph.
///
/// Complexity: O(V + E)
pub fn find_articulation_points(graph: &DiGraph<String, u32>) -> Vec<ArticulationPoint> {
    let n = graph.node_count();
    if n < 3 {
        return Vec::new();
    }

    // Build index mapping: NodeIndex → sequential usize
    let node_list: Vec<NodeIndex> = graph.node_indices().collect();
    let idx_map: HashMap<NodeIndex, usize> = node_list
        .iter()
        .enumerate()
        .map(|(i, &ni)| (ni, i))
        .collect();

    // Build undirected adjacency list (deduplicated)
    let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for edge in graph.edge_references() {
        let u = idx_map[&edge.source()];
        let v = idx_map[&edge.target()];
        if u != v {
            adj[u].insert(v);
            adj[v].insert(u);
        }
    }
    // Convert to Vec for indexed access
    let adj_vec: Vec<Vec<usize>> = adj.into_iter().map(|s| s.into_iter().collect()).collect();

    let mut disc = vec![0u32; n];
    let mut low = vec![0u32; n];
    let mut parent = vec![usize::MAX; n]; // MAX = no parent
    let mut visited = vec![false; n];
    let mut ap_set = HashSet::new();
    let mut timer = 1u32;

    for start in 0..n {
        if !visited[start] {
            tarjan_iterative(
                start,
                &adj_vec,
                &mut disc,
                &mut low,
                &mut parent,
                &mut visited,
                &mut ap_set,
                &mut timer,
            );
        }
    }

    // Convert to ArticulationPoint structs
    ap_set
        .into_iter()
        .map(|i| {
            let ni = node_list[i];
            let fan_in = graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
                .count();
            let fan_out = graph
                .neighbors_directed(ni, petgraph::Direction::Outgoing)
                .count();
            // Count DFS children to approximate bridged components
            let children_count = adj_vec[i].iter().filter(|&&c| parent[c] == i).count();
            ArticulationPoint {
                module_name: graph[ni].clone(),
                components_bridged: children_count.max(1),
                fan_in,
                fan_out,
            }
        })
        .collect()
}

/// Iterative Tarjan's articulation point algorithm.
///
/// Avoids stack overflow on large graphs by using an explicit stack.
#[allow(clippy::too_many_arguments)]
fn tarjan_iterative(
    start: usize,
    adj: &[Vec<usize>],
    disc: &mut [u32],
    low: &mut [u32],
    parent: &mut [usize],
    visited: &mut [bool],
    ap_set: &mut HashSet<usize>,
    timer: &mut u32,
) {
    // Stack entries: (node, neighbor_list_index, child_count)
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();
    visited[start] = true;
    disc[start] = *timer;
    low[start] = *timer;
    *timer += 1;
    stack.push((start, 0, 0));

    while let Some(frame) = stack.last_mut() {
        let u = frame.0;
        let ni = frame.1;

        if ni < adj[u].len() {
            let v = adj[u][ni];
            frame.1 += 1; // advance neighbor iterator

            if !visited[v] {
                visited[v] = true;
                parent[v] = u;
                disc[v] = *timer;
                low[v] = *timer;
                *timer += 1;
                stack.push((v, 0, 0));
            } else if parent[u] == usize::MAX || v != parent[u] {
                // Back edge — update low-link
                low[u] = low[u].min(disc[v]);
            }
        } else {
            // All neighbors processed — backtrack
            let u = frame.0;
            stack.pop();

            if let Some(parent_frame) = stack.last_mut() {
                let p = parent_frame.0;
                low[p] = low[p].min(low[u]);
                parent_frame.2 += 1; // increment child count

                // Articulation point conditions:
                if parent[p] == usize::MAX {
                    // Root: AP if 2+ children
                    if parent_frame.2 >= 2 {
                        ap_set.insert(p);
                    }
                } else if low[u] >= disc[p] {
                    // Non-root: AP if no back edge from subtree goes above p
                    ap_set.insert(p);
                }
            }
        }
    }
}

// ── Critical Path Finding ──

/// Finds the top-K longest weighted dependency chains.
///
/// Algorithm:
/// 1. Build condensation DAG from SCCs (using kosaraju_scc)
/// 2. Topological sort via Kahn's algorithm
/// 3. DP: longest weighted path from each source
/// 4. Reconstruct top-K paths, expanding SCC nodes to representatives
///
/// Complexity: O(V + E)
pub fn find_critical_paths(graph: &DiGraph<String, u32>, top_k: usize) -> Vec<CascadePath> {
    let n = graph.node_count();
    if n < 2 || top_k == 0 {
        return Vec::new();
    }

    // Build condensation: SCC → single super-node
    let sccs = kosaraju_scc(graph);
    let mut node_to_scc: HashMap<NodeIndex, usize> = HashMap::new();
    for (scc_idx, scc) in sccs.iter().enumerate() {
        for &node in scc {
            node_to_scc.insert(node, scc_idx);
        }
    }

    let scc_count = sccs.len();
    if scc_count < 2 {
        // Single SCC = everything is one cycle, no meaningful chain
        return Vec::new();
    }

    // Build condensation adjacency with max edge weights
    let mut cond_adj: Vec<HashMap<usize, u32>> = vec![HashMap::new(); scc_count];
    for edge in graph.edge_references() {
        let src_scc = node_to_scc[&edge.source()];
        let tgt_scc = node_to_scc[&edge.target()];
        if src_scc != tgt_scc {
            let entry = cond_adj[src_scc].entry(tgt_scc).or_insert(0);
            *entry = (*entry).max(*edge.weight());
        }
    }

    // Topological sort (Kahn's algorithm)
    let mut in_degree = vec![0usize; scc_count];
    for adj in &cond_adj {
        for &target in adj.keys() {
            in_degree[target] += 1;
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|&(_, d)| *d == 0)
        .map(|(i, _)| i)
        .collect();

    let mut topo_order = Vec::with_capacity(scc_count);
    while let Some(u) = queue.pop_front() {
        topo_order.push(u);
        for &v in cond_adj[u].keys() {
            in_degree[v] -= 1;
            if in_degree[v] == 0 {
                queue.push_back(v);
            }
        }
    }

    // DP: longest path from each SCC node
    let mut dist = vec![0u32; scc_count];
    let mut pred = vec![usize::MAX; scc_count]; // MAX = no predecessor

    for &u in &topo_order {
        for (&v, &w) in &cond_adj[u] {
            let new_dist = dist[u] + w;
            if new_dist > dist[v] {
                dist[v] = new_dist;
                pred[v] = u;
            }
        }
    }

    // Extract top-K paths by terminal weight
    let mut endpoints: Vec<(usize, u32)> = dist
        .iter()
        .enumerate()
        .filter(|&(_, d)| *d > 0)
        .map(|(i, &d)| (i, d))
        .collect();
    endpoints.sort_by(|a, b| b.1.cmp(&a.1));
    endpoints.truncate(top_k);

    endpoints
        .into_iter()
        .map(|(end, total_weight)| {
            // Reconstruct path by following predecessors
            let mut scc_path = vec![end];
            let mut current = end;
            while pred[current] != usize::MAX {
                scc_path.push(pred[current]);
                current = pred[current];
            }
            scc_path.reverse();

            // Expand SCCs to representative module names (highest-degree node)
            let chain: Vec<String> = scc_path
                .iter()
                .map(|&scc_idx| {
                    let scc = &sccs[scc_idx];
                    let representative = scc
                        .iter()
                        .max_by_key(|&&ni| {
                            graph
                                .neighbors_directed(ni, petgraph::Direction::Incoming)
                                .count()
                                + graph
                                    .neighbors_directed(ni, petgraph::Direction::Outgoing)
                                    .count()
                        })
                        .copied()
                        .unwrap_or(scc[0]);
                    graph[representative].clone()
                })
                .collect();

            let depth = chain.len();
            CascadePath {
                chain,
                total_weight,
                depth,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chain_graph() -> DiGraph<String, u32> {
        // A -> B -> C -> D (A imports B, B imports C, C imports D)
        let mut g = DiGraph::new();
        let a = g.add_node("A".into());
        let b = g.add_node("B".into());
        let c = g.add_node("C".into());
        let d = g.add_node("D".into());
        g.add_edge(a, b, 1);
        g.add_edge(b, c, 1);
        g.add_edge(c, d, 1);
        g
    }

    fn make_star_graph() -> DiGraph<String, u32> {
        // Hub imports A, B, C, D
        let mut g = DiGraph::new();
        let hub = g.add_node("hub".into());
        let a = g.add_node("A".into());
        let b = g.add_node("B".into());
        let c = g.add_node("C".into());
        let d = g.add_node("D".into());
        g.add_edge(hub, a, 1);
        g.add_edge(hub, b, 1);
        g.add_edge(hub, c, 1);
        g.add_edge(hub, d, 1);
        g
    }

    fn make_diamond_graph() -> DiGraph<String, u32> {
        // A -> B, A -> C, B -> D, C -> D
        let mut g = DiGraph::new();
        let a = g.add_node("A".into());
        let b = g.add_node("B".into());
        let c = g.add_node("C".into());
        let d = g.add_node("D".into());
        g.add_edge(a, b, 1);
        g.add_edge(a, c, 1);
        g.add_edge(b, d, 1);
        g.add_edge(c, d, 1);
        g
    }

    #[test]
    fn test_blast_radius_empty_graph() {
        let g: DiGraph<String, u32> = DiGraph::new();
        let report = compute_blast_radius_report(&g, 5);
        assert!(report.impacts.is_empty());
        assert!(report.articulation_points.is_empty());
        assert_eq!(report.summary.articulation_point_count, 0);
        assert_eq!(report.summary.max_blast_score, 0.0);
    }

    #[test]
    fn test_blast_radius_single_node() {
        let mut g = DiGraph::new();
        g.add_node("A".into());
        let report = compute_blast_radius_report(&g, 5);
        assert_eq!(report.impacts.len(), 1);
        assert_eq!(report.impacts[0].blast_score, 0.0);
        assert_eq!(report.impacts[0].downstream_count, 0);
    }

    #[test]
    fn test_blast_radius_chain() {
        let g = make_chain_graph();
        let report = compute_blast_radius_report(&g, 5);
        assert_eq!(report.impacts.len(), 4);

        // In chain A->B->C->D: A imports B, B imports C, C imports D
        // Change in D: affects C (dist 1), B (dist 2), A (dist 3)
        // Change in A: affects nobody (no one imports A)
        let d_impact = report
            .impacts
            .iter()
            .find(|m| m.module_name == "D")
            .unwrap();
        assert_eq!(d_impact.downstream_count, 3);
        assert!(d_impact.blast_score > 0.0);

        let a_impact = report
            .impacts
            .iter()
            .find(|m| m.module_name == "A")
            .unwrap();
        assert_eq!(a_impact.downstream_count, 0);
        assert_eq!(a_impact.blast_score, 0.0);
    }

    #[test]
    fn test_blast_radius_star() {
        let g = make_star_graph();
        let report = compute_blast_radius_report(&g, 5);

        // Hub->A,B,C,D: Hub imports all. Change in A affects Hub (dist 1).
        let a_impact = report
            .impacts
            .iter()
            .find(|m| m.module_name == "A")
            .unwrap();
        assert_eq!(a_impact.downstream_count, 1); // only hub

        // Change in Hub affects nobody (Hub has no incoming edges)
        let hub_impact = report
            .impacts
            .iter()
            .find(|m| m.module_name == "hub")
            .unwrap();
        assert_eq!(hub_impact.downstream_count, 0);
    }

    #[test]
    fn test_blast_radius_distance_decay() {
        let g = make_chain_graph();
        let d = g.node_indices().find(|&ni| g[ni] == "D").unwrap();
        let blast = compute_single_node_blast(&g, d);
        assert_eq!(blast.len(), 3);

        // Closer nodes should have higher impact than distant ones
        let dist1: Vec<_> = blast.iter().filter(|(_, d, _)| *d == 1).collect();
        let dist3: Vec<_> = blast.iter().filter(|(_, d, _)| *d == 3).collect();
        assert!(!dist1.is_empty());
        assert!(!dist3.is_empty());
        assert!(
            dist1[0].2 > dist3[0].2,
            "Closer nodes should have higher impact"
        );
    }

    #[test]
    fn test_weighted_edges_affect_blast() {
        let mut g = DiGraph::new();
        let a = g.add_node("A".into());
        let b = g.add_node("B".into());
        let c = g.add_node("C".into());
        g.add_edge(a, b, 10); // A heavily imports B
        g.add_edge(a, c, 1); // A lightly imports C

        // B's change and C's change both affect A, but B should have higher impact
        let b_blast = compute_single_node_blast(&g, b);
        let c_blast = compute_single_node_blast(&g, c);
        assert_eq!(b_blast.len(), 1);
        assert_eq!(c_blast.len(), 1);
        assert!(
            b_blast[0].2 > c_blast[0].2,
            "Heavily coupled module should have higher blast impact"
        );
    }

    #[test]
    fn test_articulation_points_chain() {
        // In A-B-C-D (undirected), B and C are articulation points
        let g = make_chain_graph();
        let aps = find_articulation_points(&g);
        let ap_names: HashSet<&str> = aps.iter().map(|a| a.module_name.as_str()).collect();
        assert!(
            ap_names.contains("B") && ap_names.contains("C"),
            "B and C should be articulation points in chain, got: {:?}",
            ap_names
        );
    }

    #[test]
    fn test_articulation_points_diamond() {
        // Diamond: A->B, A->C, B->D, C->D — no articulation points (redundant paths)
        let g = make_diamond_graph();
        let aps = find_articulation_points(&g);
        // In diamond graph with both directions, typically no AP
        // (there are 2 independent paths from A to D)
        assert!(
            aps.is_empty(),
            "Diamond graph should have no articulation points, got: {:?}",
            aps.iter().map(|a| &a.module_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_articulation_points_small_graph() {
        // Graphs with < 3 nodes have no articulation points
        let mut g = DiGraph::new();
        g.add_node("A".into());
        g.add_node("B".into());
        g.add_edge(
            g.node_indices().next().unwrap(),
            g.node_indices().nth(1).unwrap(),
            1,
        );
        let aps = find_articulation_points(&g);
        assert!(aps.is_empty());
    }

    #[test]
    fn test_critical_paths_chain() {
        let g = make_chain_graph();
        let paths = find_critical_paths(&g, 3);
        assert!(!paths.is_empty());
        // Longest chain should span all 4 nodes
        assert_eq!(paths[0].depth, 4);
        assert_eq!(paths[0].total_weight, 3); // 3 edges of weight 1
    }

    #[test]
    fn test_critical_paths_empty() {
        let g: DiGraph<String, u32> = DiGraph::new();
        let paths = find_critical_paths(&g, 5);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_critical_paths_with_cycle() {
        // A->B->C->A (cycle) + C->D (exits cycle)
        let mut g = DiGraph::new();
        let a = g.add_node("A".into());
        let b = g.add_node("B".into());
        let c = g.add_node("C".into());
        let d = g.add_node("D".into());
        g.add_edge(a, b, 1);
        g.add_edge(b, c, 1);
        g.add_edge(c, a, 1); // cycle
        g.add_edge(c, d, 2);
        let paths = find_critical_paths(&g, 3);
        // Condensation: {A,B,C} -> {D}, so path length 2
        assert!(!paths.is_empty());
        assert_eq!(paths[0].depth, 2);
    }

    #[test]
    fn test_blast_radius_report_summary() {
        let g = make_chain_graph();
        let report = compute_blast_radius_report(&g, 5);
        assert_eq!(report.summary.most_impactful_module, "D");
        assert!(report.summary.max_blast_score > 0.0);
        assert!(report.summary.mean_blast_score >= 0.0);
        assert!(report.summary.longest_chain_depth > 0);
    }

    #[test]
    fn test_single_node_blast_empty() {
        let mut g = DiGraph::new();
        let a = g.add_node("A".into());
        let result = compute_single_node_blast(&g, a);
        assert!(result.is_empty());
    }
}
