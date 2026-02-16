// Typed API client wrappers for the SerialAgent gateway.

const BASE = "";

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) throw new Error(`GET ${path}: ${res.status}`);
  return res.json();
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`POST ${path}: ${res.status}`);
  return res.json();
}

// ── Types ──────────────────────────────────────────────────────────

export type SessionOrigin = {
  channel?: string;
  account?: string;
  peer?: string;
  group?: string;
};

export type SessionEntry = {
  session_key: string;
  session_id: string;
  created_at: string;
  updated_at: string;
  model?: string;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  context_tokens: number;
  sm_session_id?: string;
  origin: SessionOrigin;
  running?: boolean;
};

export type SessionsListResponse = {
  sessions: SessionEntry[];
  count: number;
};

export type SessionDetailResponse = SessionEntry & {
  running: boolean;
  tokens: {
    input: number;
    output: number;
    total: number;
    context: number;
  };
};

export type TranscriptLine = {
  timestamp: string;
  role: string;
  content: string;
  metadata?: Record<string, unknown>;
};

export type TranscriptResponse = {
  session_key: string;
  session_id: string;
  total: number;
  offset: number;
  count: number;
  lines: TranscriptLine[];
};

export type NodeInfo = {
  node_id: string;
  node_type: string;
  name: string;
  capabilities: string[];
  version: string;
  tags: string[];
  session_id: string;
  connected_at: string;
  last_seen: string;
};

export type NodesListResponse = {
  nodes: NodeInfo[];
  count: number;
};

export type AgentInfo = {
  id: string;
  tools_allow?: string[];
  tools_deny?: string[];
  effective_tools_count?: number;
  resolved_executor?: string;
  models?: Record<string, string>;
  memory_mode?: string;
  limits?: {
    max_depth: number;
    max_children_per_turn: number;
    max_duration_ms: number;
  };
  compaction_enabled?: boolean;
};

export type AgentsListResponse = {
  agents: AgentInfo[];
  count: number;
};

export type ReadinessResponse = {
  ready: boolean;
  startup_policy: string;
  providers: unknown[];
  errors: string[];
};

export type ToolInvokeRequest = {
  tool: string;
  args: Record<string, unknown>;
  session_key?: string;
  timeout_ms?: number;
};

export type ToolRouteInfo = {
  kind: "local" | "node" | "unknown";
  node_id?: string;
  capability?: string;
};

export type ToolInvokeResponse = {
  request_id: string;
  ok: boolean;
  route: ToolRouteInfo;
  result?: unknown;
  error?: { kind: string; message: string };
  duration_ms: number;
};

// ── API functions ──────────────────────────────────────────────────

export const api = {
  readiness: () => get<ReadinessResponse>("/v1/models/readiness"),
  nodes: () => get<NodesListResponse>("/v1/nodes"),
  agents: () => get<AgentsListResponse>("/v1/agents"),
  sessions: () => get<SessionsListResponse>("/v1/sessions"),
  session: (key: string) =>
    get<SessionDetailResponse>(`/v1/sessions/${encodeURIComponent(key)}`),
  transcript: (key: string, offset = 0, limit = 200) =>
    get<TranscriptResponse>(
      `/v1/sessions/${encodeURIComponent(key)}/transcript?offset=${offset}&limit=${limit}`
    ),
  invokeTool: (req: ToolInvokeRequest) =>
    post<ToolInvokeResponse>("/v1/tools/invoke", req),
};
