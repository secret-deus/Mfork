export interface ConnMetadata {
  network: string;
  type: string;
  sourceIP: string;
  sourcePort: string;
  destinationIP: string;
  destinationPort: string;
  host: string;
  process?: string;
  processPath?: string;
}

export interface ConnItem {
  id: string;
  metadata: ConnMetadata;
  upload: number;
  download: number;
  start: string;
  chains: string[];
  rule: string;
  rulePayload: string;
  /** computed client-side from consecutive snapshots (bytes/s) */
  upSpeed: number;
  downSpeed: number;
}

export interface ConnectionsSnapshot {
  downloadTotal: number;
  uploadTotal: number;
  connections: ConnItem[];
}

export interface ProxyNode {
  name: string;
  type: string;
  udp: boolean;
  history: { time: string; delay: number }[];
  /** chained outbound: this node dials through that proxy (node or group) */
  "dialer-proxy"?: string;
}

export interface ProxyGroup {
  name: string;
  type: string;
  now: string;
  all: string[];
  history: { time: string; delay: number }[];
}

export interface RuleItem {
  type: string;
  payload: string;
  proxy: string;
}

export interface RuleProvider {
  name: string;
  behavior: string;
  ruleCount: number;
  updatedAt: string;
  vehicleType: string;
}

export interface LogLine {
  type: string;
  payload: string;
  seq: number;
  at: number;
}
