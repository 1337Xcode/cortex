/**
 * Graph Data Generation Script
 *
 * Generates sample graph data representing the Cortex architecture
 * and writes it to public/graph-data.json in the IGraphData format.
 *
 * Usage:
 *   npx tsx scripts/generate-graph-data.ts
 *
 * This script can also be used to convert output from `cortex viz --export`
 * into the format expected by the visualization page. To do so, pass the
 * path to the exported JSON as the first argument:
 *
 *   npx tsx scripts/generate-graph-data.ts path/to/cortex-export.json
 */

import { writeFileSync, readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// Types matching src/types/graph.ts
interface IGraphNode {
  id: string;
  name: string;
  kind: 'module' | 'function' | 'class';
  file: string;
  connections: number;
}

interface IGraphLink {
  source: string;
  target: string;
  kind: string;
}

interface IGraphData {
  nodes: IGraphNode[];
  links: IGraphLink[];
}

/**
 * Generates sample graph data based on the actual Cortex module structure.
 */
function generateSampleData(): IGraphData {
  const nodes: IGraphNode[] = [
    // Top-level modules
    { id: 'mod:cli', name: 'cli', kind: 'module', file: 'src/cli/mod.rs', connections: 8 },
    { id: 'mod:agents', name: 'agents', kind: 'module', file: 'src/agents/mod.rs', connections: 5 },
    { id: 'mod:bundle', name: 'bundle', kind: 'module', file: 'src/bundle/mod.rs', connections: 4 },
    { id: 'mod:indexer', name: 'indexer', kind: 'module', file: 'src/indexer/mod.rs', connections: 6 },
    { id: 'mod:mcp', name: 'mcp', kind: 'module', file: 'src/mcp/mod.rs', connections: 7 },
    { id: 'mod:memory', name: 'memory', kind: 'module', file: 'src/memory/mod.rs', connections: 4 },
    { id: 'mod:security', name: 'security', kind: 'module', file: 'src/security/mod.rs', connections: 3 },
    { id: 'mod:store', name: 'store', kind: 'module', file: 'src/store/mod.rs', connections: 7 },
    { id: 'mod:watcher', name: 'watcher', kind: 'module', file: 'src/watcher/mod.rs', connections: 3 },

    // Key functions from cli
    { id: 'fn:cli:run', name: 'run', kind: 'function', file: 'src/cli/mod.rs', connections: 6 },
    { id: 'fn:cli:parse_args', name: 'parse_args', kind: 'function', file: 'src/cli/mod.rs', connections: 2 },

    // Key functions from agents
    { id: 'fn:agents:configure', name: 'configure_agent', kind: 'function', file: 'src/agents/mod.rs', connections: 3 },
    { id: 'fn:agents:detect', name: 'detect_agents', kind: 'function', file: 'src/agents/mod.rs', connections: 2 },

    // Key functions from indexer
    { id: 'fn:indexer:index_repo', name: 'index_repo', kind: 'function', file: 'src/indexer/mod.rs', connections: 5 },
    { id: 'fn:indexer:parse_file', name: 'parse_file', kind: 'function', file: 'src/indexer/mod.rs', connections: 4 },
    { id: 'fn:indexer:build_graph', name: 'build_graph', kind: 'function', file: 'src/indexer/mod.rs', connections: 3 },

    // Key functions from mcp
    { id: 'fn:mcp:serve', name: 'serve', kind: 'function', file: 'src/mcp/mod.rs', connections: 5 },
    { id: 'fn:mcp:handle_request', name: 'handle_request', kind: 'function', file: 'src/mcp/mod.rs', connections: 4 },
    { id: 'fn:mcp:register_tools', name: 'register_tools', kind: 'function', file: 'src/mcp/mod.rs', connections: 3 },

    // Key functions from memory
    { id: 'fn:memory:store_context', name: 'store_context', kind: 'function', file: 'src/memory/mod.rs', connections: 3 },
    { id: 'fn:memory:recall', name: 'recall', kind: 'function', file: 'src/memory/mod.rs', connections: 2 },

    // Key functions from security
    { id: 'fn:security:scan', name: 'scan_vulnerabilities', kind: 'function', file: 'src/security/mod.rs', connections: 2 },
    { id: 'fn:security:analyze', name: 'analyze_dependencies', kind: 'function', file: 'src/security/mod.rs', connections: 2 },

    // Key functions from store
    { id: 'fn:store:init_db', name: 'init_db', kind: 'function', file: 'src/store/mod.rs', connections: 4 },
    { id: 'fn:store:query', name: 'query_graph', kind: 'function', file: 'src/store/mod.rs', connections: 5 },
    { id: 'fn:store:insert', name: 'insert_node', kind: 'function', file: 'src/store/mod.rs', connections: 3 },

    // Key functions from watcher
    { id: 'fn:watcher:watch', name: 'watch_files', kind: 'function', file: 'src/watcher/mod.rs', connections: 3 },
    { id: 'fn:watcher:on_change', name: 'on_change', kind: 'function', file: 'src/watcher/mod.rs', connections: 2 },

    // Key functions from bundle
    { id: 'fn:bundle:package', name: 'package_binary', kind: 'function', file: 'src/bundle/mod.rs', connections: 2 },
    { id: 'fn:bundle:install', name: 'install', kind: 'function', file: 'src/bundle/mod.rs', connections: 3 },
  ];

  const links: IGraphLink[] = [
    // CLI calls into other modules
    { source: 'fn:cli:run', target: 'mod:indexer', kind: 'call' },
    { source: 'fn:cli:run', target: 'mod:mcp', kind: 'call' },
    { source: 'fn:cli:run', target: 'mod:agents', kind: 'call' },
    { source: 'fn:cli:run', target: 'mod:watcher', kind: 'call' },
    { source: 'fn:cli:run', target: 'mod:security', kind: 'call' },
    { source: 'fn:cli:run', target: 'mod:bundle', kind: 'call' },
    { source: 'mod:cli', target: 'fn:cli:run', kind: 'contains' },
    { source: 'mod:cli', target: 'fn:cli:parse_args', kind: 'contains' },

    // Indexer relationships
    { source: 'fn:indexer:index_repo', target: 'fn:indexer:parse_file', kind: 'call' },
    { source: 'fn:indexer:index_repo', target: 'fn:indexer:build_graph', kind: 'call' },
    { source: 'fn:indexer:index_repo', target: 'fn:store:insert', kind: 'call' },
    { source: 'fn:indexer:build_graph', target: 'fn:store:insert', kind: 'call' },
    { source: 'mod:indexer', target: 'fn:indexer:index_repo', kind: 'contains' },
    { source: 'mod:indexer', target: 'fn:indexer:parse_file', kind: 'contains' },
    { source: 'mod:indexer', target: 'fn:indexer:build_graph', kind: 'contains' },

    // MCP relationships
    { source: 'fn:mcp:serve', target: 'fn:mcp:handle_request', kind: 'call' },
    { source: 'fn:mcp:serve', target: 'fn:mcp:register_tools', kind: 'call' },
    { source: 'fn:mcp:handle_request', target: 'fn:store:query', kind: 'call' },
    { source: 'fn:mcp:handle_request', target: 'fn:memory:recall', kind: 'call' },
    { source: 'mod:mcp', target: 'fn:mcp:serve', kind: 'contains' },
    { source: 'mod:mcp', target: 'fn:mcp:handle_request', kind: 'contains' },
    { source: 'mod:mcp', target: 'fn:mcp:register_tools', kind: 'contains' },

    // Memory relationships
    { source: 'fn:memory:store_context', target: 'fn:store:insert', kind: 'call' },
    { source: 'fn:memory:recall', target: 'fn:store:query', kind: 'call' },
    { source: 'mod:memory', target: 'fn:memory:store_context', kind: 'contains' },
    { source: 'mod:memory', target: 'fn:memory:recall', kind: 'contains' },

    // Security relationships
    { source: 'fn:security:scan', target: 'fn:store:query', kind: 'call' },
    { source: 'fn:security:analyze', target: 'fn:indexer:parse_file', kind: 'call' },
    { source: 'mod:security', target: 'fn:security:scan', kind: 'contains' },
    { source: 'mod:security', target: 'fn:security:analyze', kind: 'contains' },

    // Store relationships
    { source: 'fn:store:init_db', target: 'fn:store:insert', kind: 'call' },
    { source: 'mod:store', target: 'fn:store:init_db', kind: 'contains' },
    { source: 'mod:store', target: 'fn:store:query', kind: 'contains' },
    { source: 'mod:store', target: 'fn:store:insert', kind: 'contains' },

    // Watcher relationships
    { source: 'fn:watcher:watch', target: 'fn:watcher:on_change', kind: 'call' },
    { source: 'fn:watcher:on_change', target: 'fn:indexer:index_repo', kind: 'call' },
    { source: 'mod:watcher', target: 'fn:watcher:watch', kind: 'contains' },
    { source: 'mod:watcher', target: 'fn:watcher:on_change', kind: 'contains' },

    // Agents relationships
    { source: 'fn:agents:configure', target: 'fn:agents:detect', kind: 'call' },
    { source: 'mod:agents', target: 'fn:agents:configure', kind: 'contains' },
    { source: 'mod:agents', target: 'fn:agents:detect', kind: 'contains' },

    // Bundle relationships
    { source: 'fn:bundle:install', target: 'fn:agents:configure', kind: 'call' },
    { source: 'mod:bundle', target: 'fn:bundle:package', kind: 'contains' },
    { source: 'mod:bundle', target: 'fn:bundle:install', kind: 'contains' },
  ];

  return { nodes, links };
}

/**
 * Converts cortex viz --export output to IGraphData format.
 * The cortex export format may differ, so this handles common variations.
 */
function convertCortexExport(exportPath: string): IGraphData {
  const raw = JSON.parse(readFileSync(exportPath, 'utf-8'));

  // If the export already matches IGraphData format, return as-is
  if (raw.nodes && raw.links && Array.isArray(raw.nodes)) {
    return raw as IGraphData;
  }

  // Handle potential alternative export formats
  const nodes: IGraphNode[] = (raw.nodes || raw.vertices || []).map((n: any) => ({
    id: n.id || n.name,
    name: n.name || n.label || n.id,
    kind: n.kind || n.type || 'function',
    file: n.file || n.path || '',
    connections: n.connections || n.degree || 0,
  }));

  const links: IGraphLink[] = (raw.links || raw.edges || []).map((e: any) => ({
    source: e.source || e.from,
    target: e.target || e.to,
    kind: e.kind || e.type || 'call',
  }));

  return { nodes, links };
}

// Main execution
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const outputPath = resolve(__dirname, '..', 'public', 'graph-data.json');

const exportArg = process.argv[2];

let data: IGraphData;

if (exportArg && existsSync(exportArg)) {
  console.log(`Converting cortex export from: ${exportArg}`);
  data = convertCortexExport(exportArg);
} else {
  console.log('Generating sample graph data from Cortex architecture...');
  data = generateSampleData();
}

writeFileSync(outputPath, JSON.stringify(data, null, 2), 'utf-8');
console.log(`Graph data written to: ${outputPath}`);
console.log(`  Nodes: ${data.nodes.length}`);
console.log(`  Links: ${data.links.length}`);
