//! Viz command: export or serve an interactive 3D graph visualization.
//!
//! `cortex viz --export graph.html` generates a standalone HTML file
//! with an embedded 3D force-directed graph of the codebase.
//! `cortex viz` starts the visualization server (delegates to visualizer module).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::queries::{community, graph};

/// Export a standalone HTML file with the graph visualization.
pub fn export_html(store: &Arc<StoreManager>, output_path: &Path) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    // Get all nodes and edges for the graph
    let arch = graph::get_architecture_summary(&conn)?;

    // Get community assignments for coloring
    let communities = community::detect_communities(&conn, None, 0.5)?;

    // Build node-to-community map
    let mut node_community: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, comm) in communities.communities.iter().enumerate() {
        for member in &comm.suggested_api_surface {
            node_community.insert(member.clone(), i);
        }
    }

    // Get ALL nodes (no limit, this tool is for large codebases)
    let hotspots = graph::get_hotspot_nodes(&conn, 100_000)?;

    // Build JSON data for the graph
    let nodes_json: Vec<serde_json::Value> = hotspots
        .iter()
        .map(|n| {
            let community_id = node_community.get(&n.fqn).copied().unwrap_or(0);
            serde_json::json!({
                "id": n.fqn,
                "file": n.file,
                "kind": n.kind,
                "callers": n.caller_count,
                "group": community_id,
            })
        })
        .collect();

    // Get ALL edges (no limit)
    let node_set: std::collections::HashSet<&str> =
        hotspots.iter().map(|n| n.fqn.as_str()).collect();
    let all_edges = graph::get_all_edges(&conn, 500_000)?;
    let edges_json: Vec<serde_json::Value> = all_edges
        .iter()
        .filter(|e| node_set.contains(e.caller.as_str()) && node_set.contains(e.callee.as_str()))
        .map(|e| {
            serde_json::json!({
                "source": e.caller,
                "target": e.callee,
            })
        })
        .collect();

    let graph_data = serde_json::json!({
        "nodes": nodes_json,
        "links": edges_json,
    });

    // Generate the standalone HTML
    let html = generate_standalone_html(&graph_data, &arch);

    fs::write(output_path, &html)?;
    println!("Graph exported to {}", output_path.display());
    println!("Open in a browser to explore the interactive 3D visualization.");
    println!(
        "  {} nodes, {} edges, {} communities",
        nodes_json.len(),
        edges_json.len(),
        communities.communities.len()
    );

    Ok(())
}

/// Generate a standalone HTML file with embedded 3D force graph.
/// Obsidian-quality UI with IBM Plex font, sidebar, search, node details panel.
fn generate_standalone_html(
    graph_data: &serde_json::Value,
    arch: &graph::ArchitectureSummary,
) -> String {
    let data_json = serde_json::to_string(graph_data).unwrap_or_default();
    let node_count = graph_data["nodes"].as_array().map(|a| a.len()).unwrap_or(0);
    let edge_count = graph_data["links"].as_array().map(|a| a.len()).unwrap_or(0);
    let project = arch
        .top_level_modules
        .first()
        .unwrap_or(&"project".to_string())
        .clone();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Cortex Graph | {project}</title>
<link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=IBM+Plex+Sans:wght@300;400;500;600&display=swap" rel="stylesheet">
<script src="https://unpkg.com/lucide@latest"></script>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ overflow: hidden; background: #0f0f14; font-family: 'IBM Plex Sans', sans-serif; color: #e2e4e9; }}
::-webkit-scrollbar {{ width: 0; height: 0; }}
* {{ scrollbar-width: none; }}

#graph {{ width: 100vw; height: 100vh; }}

/* Header bar */
#header {{
  position: fixed; top: 0; left: 0; right: 0; height: 48px; z-index: 100;
  background: rgba(15,15,20,0.92); backdrop-filter: blur(12px);
  border-bottom: 1px solid rgba(255,255,255,0.06);
  display: flex; align-items: center; padding: 0 20px; gap: 16px;
}}
#header h1 {{ font-size: 14px; font-weight: 500; color: #a8b1ff; letter-spacing: -0.3px; }}
#header .stats {{ font-size: 12px; color: #6b7280; font-family: 'IBM Plex Mono', monospace; }}

/* Search */
#search-container {{
  position: fixed; top: 60px; left: 16px; z-index: 90; width: 280px;
}}
#search {{
  width: 100%; padding: 10px 14px; border-radius: 8px;
  background: rgba(20,20,28,0.95); border: 1px solid rgba(255,255,255,0.08);
  color: #e2e4e9; font-size: 13px; font-family: 'IBM Plex Mono', monospace;
  outline: none; transition: border-color 0.2s;
}}
#search:focus {{ border-color: rgba(168,177,255,0.4); }}
#search::placeholder {{ color: #4b5563; }}

/* Node detail panel */
#panel {{
  position: fixed; top: 60px; right: 16px; width: 320px; z-index: 90;
  background: rgba(15,15,20,0.95); backdrop-filter: blur(12px);
  border: 1px solid rgba(255,255,255,0.06); border-radius: 12px;
  padding: 20px; display: none; max-height: calc(100vh - 80px); overflow-y: auto;
}}
#panel.visible {{ display: block; }}
#panel h2 {{ font-size: 14px; font-weight: 600; color: #a8b1ff; margin-bottom: 12px; word-break: break-all; }}
#panel .field {{ margin-bottom: 10px; }}
#panel .field-label {{ font-size: 11px; color: #6b7280; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 2px; }}
#panel .field-value {{ font-size: 13px; font-family: 'IBM Plex Mono', monospace; color: #d1d5db; }}
#panel .connections {{ margin-top: 16px; border-top: 1px solid rgba(255,255,255,0.06); padding-top: 12px; }}
#panel .conn-item {{ font-size: 12px; padding: 4px 0; color: #9ca3af; cursor: pointer; transition: color 0.15s; }}
#panel .conn-item:hover {{ color: #a8b1ff; }}
#panel .badge {{ display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 11px; font-weight: 500; }}
#panel .badge-function {{ background: rgba(88,166,255,0.15); color: #58a6ff; }}
#panel .badge-class {{ background: rgba(214,168,255,0.15); color: #d2a8ff; }}
#panel .badge-route {{ background: rgba(126,231,135,0.15); color: #7ee787; }}
#panel .badge-module {{ background: rgba(255,166,87,0.15); color: #ffa657; }}

/* Legend */
#legend {{
  position: fixed; bottom: 16px; left: 16px; z-index: 90;
  background: rgba(15,15,20,0.9); border: 1px solid rgba(255,255,255,0.06);
  border-radius: 8px; padding: 12px 16px; font-size: 11px;
}}
#legend .item {{ display: flex; align-items: center; gap: 8px; margin: 4px 0; }}
#legend .dot {{ width: 8px; height: 8px; border-radius: 50%; }}

/* Controls hint */
#controls {{
  position: fixed; bottom: 16px; right: 16px; z-index: 90;
  font-size: 11px; color: #4b5563; text-align: right;
}}
</style>
</head>
<body>

<div id="header">
  <h1>Cortex Graph</h1>
  <span class="stats">{node_count} nodes, {edge_count} edges</span>
  <span class="stats" id="top-nodes" style="margin-left:auto;cursor:pointer;color:#a8b1ff" onclick="showTopNodes()"><i data-lucide="flame" style="width:12px;height:12px;display:inline-block;vertical-align:middle;margin-right:4px"></i>Show hotspots</span>
</div>

<div id="search-container">
  <input id="search" type="text" placeholder="Search symbols..." autocomplete="off" spellcheck="false">
</div>

<div id="panel">
  <h2 id="panel-name"></h2>
  <div class="field"><div class="field-label">Kind</div><div class="field-value" id="panel-kind"></div></div>
  <div class="field"><div class="field-label">File</div><div class="field-value" id="panel-file"></div></div>
  <div class="field"><div class="field-label">Risk</div><div class="field-value" id="panel-risk"></div></div>
  <div class="connections" id="panel-connections"></div>
</div>

<div id="legend">
  <div class="item"><div class="dot" style="background:#7c8aff"></div> High connectivity</div>
  <div class="item"><div class="dot" style="background:#f78166"></div> Medium connectivity</div>
  <div class="item"><div class="dot" style="background:#7ee787"></div> Low connectivity</div>
  <div class="item" style="margin-top:8px;color:#6b7280">Node size = caller count</div>
</div>

<div id="controls">
  Scroll to zoom<br>Drag to rotate<br>Click to inspect<br>Shift+Click to focus
</div>

<div id="graph"></div>

<script src="https://unpkg.com/3d-force-graph@1"></script>
<script>
const data = {data};

const palette = [
  '#7c8aff', '#f78166', '#7ee787', '#d2a8ff', '#79c0ff',
  '#ffa657', '#ff7b72', '#a5d6ff', '#56d364', '#e6edf3',
  '#ff9ecb', '#b4f0a8', '#ffd700', '#87ceeb', '#dda0dd'
];

const nodeColor = n => palette[n.group % palette.length];

// Build adjacency for panel connections
const adjacency = {{}};
data.links.forEach(l => {{
  const src = typeof l.source === 'object' ? l.source.id : l.source;
  const tgt = typeof l.target === 'object' ? l.target.id : l.target;
  if (!adjacency[src]) adjacency[src] = {{ out: [], in: [] }};
  if (!adjacency[tgt]) adjacency[tgt] = {{ out: [], in: [] }};
  adjacency[src].out.push(tgt);
  adjacency[tgt].in.push(src);
}});

let highlightNodes = new Set();
let highlightLinks = new Set();
let selectedNode = null;

const Graph = ForceGraph3D()(document.getElementById('graph'))
  .graphData(data)
  .backgroundColor('#0f0f14')
  .nodeColor(n => {{
    if (highlightNodes.size > 0) {{
      return highlightNodes.has(n.id) ? nodeColor(n) : 'rgba(60,60,80,0.3)';
    }}
    return nodeColor(n);
  }})
  .nodeVal(n => Math.max(2, Math.pow((n.callers || 0) + 1, 0.6) * 2))
  .nodeOpacity(0.92)
  .linkColor(link => {{
    if (highlightLinks.size > 0) {{
      return highlightLinks.has(link) ? 'rgba(168,177,255,0.7)' : 'rgba(40,40,60,0.06)';
    }}
    return 'rgba(100,110,140,0.2)';
  }})
  .linkWidth(link => highlightLinks.has(link) ? 2 : 0.8)
  .linkDirectionalParticles(link => highlightLinks.has(link) ? 2 : 0)
  .linkDirectionalParticleWidth(1.5)
  .linkDirectionalParticleColor(() => '#a8b1ff')
  .onNodeClick((node, event) => {{
    selectedNode = node;
    showPanel(node);
    highlightConnections(node);
    Graph.nodeColor(Graph.nodeColor());
    Graph.linkColor(Graph.linkColor());
    Graph.linkWidth(Graph.linkWidth());
  }})
  .onBackgroundClick(() => {{
    selectedNode = null;
    highlightNodes.clear();
    highlightLinks.clear();
    document.getElementById('panel').classList.remove('visible');
    Graph.nodeColor(Graph.nodeColor());
    Graph.linkColor(Graph.linkColor());
    Graph.linkWidth(Graph.linkWidth());
  }});

function highlightConnections(node) {{
  highlightNodes.clear();
  highlightLinks.clear();
  highlightNodes.add(node.id);

  const adj = adjacency[node.id];
  if (adj) {{
    adj.in.forEach(id => highlightNodes.add(id));
    adj.out.forEach(id => highlightNodes.add(id));
  }}

  data.links.forEach(link => {{
    const src = typeof link.source === 'object' ? link.source.id : link.source;
    const tgt = typeof link.target === 'object' ? link.target.id : link.target;
    if (src === node.id || tgt === node.id) {{
      highlightLinks.add(link);
    }}
  }});
}}

function showPanel(node, keepExpanded) {{
  if (!keepExpanded) {{
    window._expandedIn = false;
    window._expandedOut = false;
  }}
  const panel = document.getElementById('panel');
  panel.classList.add('visible');
  document.getElementById('panel-name').textContent = node.id.split('::').pop() || node.id;
  document.getElementById('panel-kind').innerHTML = '<span class="badge badge-' + (node.kind || 'function').toLowerCase() + '">' + (node.kind || 'Function') + '</span>';
  document.getElementById('panel-file').textContent = node.file || 'unknown';

  // Risk assessment based on caller count
  const callers = node.callers || 0;
  const adj = adjacency[node.id] || {{ in: [], out: [] }};
  let riskLevel, riskColor;
  if (callers >= 10) {{ riskLevel = 'High'; riskColor = '#f78166'; }}
  else if (callers >= 4) {{ riskLevel = 'Medium'; riskColor = '#ffa657'; }}
  else {{ riskLevel = 'Low'; riskColor = '#7ee787'; }}
  document.getElementById('panel-risk').innerHTML = '<span style="color:' + riskColor + '">' + riskLevel + ' blast radius</span> (' + callers + ' callers, ' + adj.out.length + ' dependencies)';

  const connDiv = document.getElementById('panel-connections');
  let html = '';
  if (adj.in.length > 0) {{
    html += '<div class="field-label" style="margin-bottom:6px"><i data-lucide="arrow-down-left" style="width:11px;height:11px;display:inline-block;vertical-align:middle;margin-right:4px"></i>Called by (' + adj.in.length + ')</div>';
    const showAllIn = adj.in.length <= 12 || window._expandedIn;
    const inSlice = showAllIn ? adj.in : adj.in.slice(0, 8);
    inSlice.forEach(id => {{
      html += '<div class="conn-item" onclick="focusNode(\'' + id.replace(/'/g, "\\'") + '\')"><i data-lucide="corner-down-right" style="width:11px;height:11px;display:inline-block;vertical-align:middle;margin-right:4px;opacity:0.5"></i>' + id.split('::').pop() + '</div>';
    }});
    if (!showAllIn && adj.in.length > 8) {{
      html += '<div class="conn-item" style="color:#a8b1ff;cursor:pointer" onclick="window._expandedIn=true;showPanel(selectedNode,true)"><i data-lucide="chevrons-down" style="width:11px;height:11px;display:inline-block;vertical-align:middle;margin-right:4px"></i>show ' + (adj.in.length - 8) + ' more</div>';
    }}
  }}
  if (adj.out.length > 0) {{
    html += '<div class="field-label" style="margin-top:12px;margin-bottom:6px"><i data-lucide="arrow-up-right" style="width:11px;height:11px;display:inline-block;vertical-align:middle;margin-right:4px"></i>Calls (' + adj.out.length + ')</div>';
    const showAllOut = adj.out.length <= 12 || window._expandedOut;
    const outSlice = showAllOut ? adj.out : adj.out.slice(0, 8);
    outSlice.forEach(id => {{
      html += '<div class="conn-item" onclick="focusNode(\'' + id.replace(/'/g, "\\'") + '\')"><i data-lucide="corner-down-right" style="width:11px;height:11px;display:inline-block;vertical-align:middle;margin-right:4px;opacity:0.5"></i>' + id.split('::').pop() + '</div>';
    }});
    if (!showAllOut && adj.out.length > 8) {{
      html += '<div class="conn-item" style="color:#a8b1ff;cursor:pointer" onclick="window._expandedOut=true;showPanel(selectedNode,true)"><i data-lucide="chevrons-down" style="width:11px;height:11px;display:inline-block;vertical-align:middle;margin-right:4px"></i>show ' + (adj.out.length - 8) + ' more</div>';
    }}
  }}
  if (adj.in.length === 0 && adj.out.length === 0) {{
    html = '<div style="color:#4b5563;font-size:12px">No connections</div>';
  }}
  connDiv.innerHTML = html;
  lucide.createIcons();
}}

function focusNode(id) {{
  const node = data.nodes.find(n => n.id === id);
  if (node) {{
    Graph.cameraPosition(
      {{ x: node.x + 100, y: node.y + 50, z: node.z + 100 }},
      {{ x: node.x, y: node.y, z: node.z }},
      1000
    );
    setTimeout(() => {{
      selectedNode = node;
      showPanel(node);
      highlightConnections(node);
      Graph.nodeColor(Graph.nodeColor());
      Graph.linkColor(Graph.linkColor());
    }}, 500);
  }}
}}

// Search
const searchInput = document.getElementById('search');
searchInput.addEventListener('input', (e) => {{
  const query = e.target.value.toLowerCase();
  if (query.length < 2) {{
    highlightNodes.clear();
    highlightLinks.clear();
    Graph.nodeColor(Graph.nodeColor());
    return;
  }}
  highlightNodes.clear();
  data.nodes.forEach(n => {{
    if (n.id.toLowerCase().includes(query)) {{
      highlightNodes.add(n.id);
    }}
  }});
  Graph.nodeColor(Graph.nodeColor());
}});

searchInput.addEventListener('keydown', (e) => {{
  if (e.key === 'Enter' && highlightNodes.size > 0) {{
    const firstId = [...highlightNodes][0];
    focusNode(firstId);
  }}
}});

// Show top hotspot nodes (highest caller count)
function showTopNodes() {{
  const sorted = [...data.nodes].sort((a, b) => (b.callers || 0) - (a.callers || 0));
  const top = sorted.slice(0, 5);
  highlightNodes.clear();
  top.forEach(n => highlightNodes.add(n.id));
  Graph.nodeColor(Graph.nodeColor());

  // Focus camera on the top node
  if (top[0]) {{
    focusNode(top[0].id);
  }}
}}

// Initialize Lucide icons
lucide.createIcons();
</script>
</body>
</html>"#,
        project = project,
        node_count = node_count,
        edge_count = edge_count,
        data = data_json,
    )
}
