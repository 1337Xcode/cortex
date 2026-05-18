export interface IGraphNode {
  id: string;
  name: string;
  kind: 'module' | 'function' | 'class';
  file: string;
  connections: number;
}

export interface IGraphLink {
  source: string;
  target: string;
  kind: string;
}

export interface IGraphData {
  nodes: IGraphNode[];
  links: IGraphLink[];
}
