use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::types::CodeIndex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub file_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub edge_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImpactReport {
    pub root_symbol: String,
    pub affected_symbols: Vec<String>,
    pub affected_files: Vec<String>,
    pub max_depth: usize,
}

pub fn build_code_graph(index: &CodeIndex) -> CodeGraph {
    let mut graph = CodeGraph::default();

    for symbol in &index.symbols {
        graph.nodes.push(GraphNode {
            id: symbol.name.clone(),
            label: format!("{} ({})", symbol.name, symbol.symbol_type.as_str()),
            file_path: symbol.file_path.clone(),
            kind: symbol.symbol_type.as_str().to_owned(),
        });
    }

    for edge in &index.call_edges {
        graph.edges.push(GraphEdge {
            from: edge.caller_symbol.clone(),
            to: edge.callee_symbol.clone(),
            edge_type: "calls".to_owned(),
        });
    }

    for (from_file, to_dep) in &index.dependencies {
        graph.edges.push(GraphEdge {
            from: format!("file:{}", from_file),
            to: format!("dep:{}", to_dep),
            edge_type: "imports".to_owned(),
        });
    }

    graph
}

pub fn analyze_impact(index: &CodeIndex, root_symbol: &str, max_depth: usize) -> ImpactReport {
    let reverse_calls = build_reverse_call_map(index);
    let symbol_to_file = index
        .symbols
        .iter()
        .map(|symbol| (symbol.name.clone(), symbol.file_path.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back((root_symbol.to_owned(), 0usize));

    while let Some((current, depth)) = queue.pop_front() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        if let Some(callers) = reverse_calls.get(&current) {
            for caller in callers {
                queue.push_back((caller.clone(), depth + 1));
            }
        }
    }

    let affected_symbols = visited.iter().cloned().collect::<Vec<_>>();
    let mut affected_files = visited
        .iter()
        .filter_map(|symbol| symbol_to_file.get(symbol).cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    affected_files.sort();

    ImpactReport {
        root_symbol: root_symbol.to_owned(),
        affected_symbols,
        affected_files,
        max_depth,
    }
}

pub fn render_dot(graph: &CodeGraph) -> String {
    let mut lines = Vec::new();
    lines.push("digraph code_graph {".to_owned());
    lines.push("  rankdir=LR;".to_owned());

    for node in &graph.nodes {
        lines.push(format!(
            "  \"{}\" [label=\"{}\"];",
            escape(&node.id),
            escape(&node.label)
        ));
    }
    for edge in &graph.edges {
        lines.push(format!(
            "  \"{}\" -> \"{}\" [label=\"{}\"];",
            escape(&edge.from),
            escape(&edge.to),
            escape(&edge.edge_type)
        ));
    }

    lines.push("}".to_owned());
    lines.join("\n")
}

fn build_reverse_call_map(index: &CodeIndex) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::<String, Vec<String>>::new();
    for edge in &index.call_edges {
        map.entry(edge.callee_symbol.clone())
            .or_default()
            .push(edge.caller_symbol.clone());
    }
    map
}

fn escape(value: &str) -> String {
    value.replace('"', "\\\"")
}
