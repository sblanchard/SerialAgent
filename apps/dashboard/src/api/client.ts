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
  total_tools_available?: number;
};

export type ProviderCapabilities = {
  supports_tools: string;
  supports_streaming: boolean;
  supports_json_mode: boolean;
  supports_vision: boolean;
  context_window_tokens: number;
  max_output_tokens?: number;
};

export type ProviderInfo = {
  id: string;
  capabilities: ProviderCapabilities;
};

export type InitError = {
  provider_id: string;
  kind: string;
  error: string;
};

export type ReadinessResponse = {
  ready: boolean;
  provider_count: number;
  startup_policy: string;
  providers: ProviderInfo[];
  init_errors: InitError[];
  roles: Record<string, string>;
  has_executor: boolean;
  memory_configured: boolean;
  nodes_connected: number;
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

// ── Admin types ──────────────────────────────────────────────────

export type SystemInfo = {
  version: string;
  server: { host: string; port: number };
  admin_token_set: boolean;
  workspace_path: string;
  skills_path: string;
  serial_memory_url: string;
  serial_memory_transport: string;
  provider_count: number;
  node_count: number;
  session_count: number;
};

export type ScannedAgent = {
  name: string;
  has_models: boolean;
  has_auth: boolean;
  session_count: number;
  models: Record<string, string>;
};

export type ScannedWorkspace = {
  name: string;
  path: string;
  files: string[];
  total_size_bytes: number;
};

export type ScanResult = {
  path: string;
  valid: boolean;
  agents: ScannedAgent[];
  workspaces: ScannedWorkspace[];
  warnings: string[];
};

export type ImportApplyRequest = {
  path: string;
  workspaces: string[];
  agents: string[];
  import_models: boolean;
  import_auth: boolean;
  import_sessions: boolean;
};

export type ImportApplyResult = {
  success: boolean;
  workspaces_imported: string[];
  agents_imported: string[];
  sessions_imported: number;
  files_copied: number;
  warnings: string[];
  errors: string[];
};

export type WorkspaceFile = {
  name: string;
  size: number;
  sha256?: string;
  content?: string;
};

export type WorkspaceFilesResponse = {
  path: string;
  files: WorkspaceFile[];
  count: number;
};

export type SkillDetailed = {
  name: string;
  description: string;
  risk: string;
  ready: boolean;
  permission_scope?: string;
};

export type SkillsDetailedResponse = {
  skills: SkillDetailed[];
  total: number;
  ready_count: number;
};

export type SessionResetResponse = {
  session_key: string;
  session_id: string;
  reset: boolean;
};

export type SessionStopResponse = {
  session_key: string;
  was_running: boolean;
  stopped: boolean;
};

// ── Import (staging-based) types ──────────────────────────────────

export type ImportSource =
  | { local: { path: string; follow_symlinks?: boolean } }
  | {
      ssh: {
        host: string;
        user?: string;
        port?: number;
        remote_path?: string;
        strict_host_key_checking?: boolean;
        auth?: SshAuth;
      };
    };

export type SshAuth =
  | "agent"
  | { key_file: { key_path: string } }
  | { password: { password: string } };

export type ImportOptions = {
  include_workspaces?: boolean;
  include_sessions?: boolean;
  include_models?: boolean;
  include_auth_profiles?: boolean;
};

export type ImportPreviewRequest = {
  source: ImportSource;
  options?: ImportOptions;
};

export type AgentInventory = {
  agent_id: string;
  session_files: number;
  has_models_json: boolean;
  has_auth_profiles_json: boolean;
};

export type WorkspaceInventory = {
  name: string;
  rel_path: string;
  approx_files: number;
  approx_bytes: number;
};

export type ImportInventory = {
  agents: AgentInventory[];
  workspaces: WorkspaceInventory[];
  totals: { approx_files: number; approx_bytes: number };
};

export type SensitiveFile = {
  rel_path: string;
  key_paths: string[];
};

export type SensitiveReport = {
  sensitive_files: SensitiveFile[];
  redacted_samples: string[];
};

export type ConflictsHint = {
  default_workspace_dest: string;
  default_sessions_dest: string;
};

export type ImportPreviewResponse = {
  staging_id: string;
  staging_dir: string;
  inventory: ImportInventory;
  sensitive: SensitiveReport;
  conflicts_hint: ConflictsHint;
};

export type MergeStrategy = "merge_safe" | "replace" | "skip_existing";

export type ImportApplyRequestV2 = {
  staging_id: string;
  merge_strategy?: MergeStrategy;
  options?: ImportOptions;
};

export type ImportedSummary = {
  agents: string[];
  workspaces: string[];
  sessions_copied: number;
  dest_workspace_root: string;
  dest_sessions_root: string;
};

export type ImportApplyResponseV2 = {
  staging_id: string;
  imported: ImportedSummary;
  warnings: string[];
};

export type TestSshResponse = {
  ok: boolean;
  stdout?: string;
  stderr?: string;
  error?: string;
};

// ── API functions ──────────────────────────────────────────────────

export const api = {
  // Core
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
  resetSession: (key: string) =>
    post<SessionResetResponse>(`/v1/sessions/${encodeURIComponent(key)}/reset`, {}),
  stopSession: (key: string) =>
    post<SessionStopResponse>(`/v1/sessions/${encodeURIComponent(key)}/stop`, {}),
  invokeTool: (req: ToolInvokeRequest) =>
    post<ToolInvokeResponse>("/v1/tools/invoke", req),

  // Admin
  systemInfo: () => get<SystemInfo>("/v1/admin/info"),
  scanOpenClaw: (path: string) =>
    post<ScanResult>("/v1/admin/import/openclaw/scan", { path }),
  applyOpenClawImport: (req: ImportApplyRequest) =>
    post<ImportApplyResult>("/v1/admin/import/openclaw/apply", req),
  workspaceFiles: () => get<WorkspaceFilesResponse>("/v1/admin/workspace/files"),
  skillsDetailed: () => get<SkillsDetailedResponse>("/v1/admin/skills"),

  // Import (staging-based)
  importPreview: (req: ImportPreviewRequest) =>
    post<ImportPreviewResponse>("/v1/import/openclaw/preview", req),
  importApply: (req: ImportApplyRequestV2) =>
    post<ImportApplyResponseV2>("/v1/import/openclaw/apply", req),
  testSsh: (host: string, user?: string, port?: number) =>
    post<TestSshResponse>("/v1/import/openclaw/test-ssh", { host, user, port }),

  // Provider listing
  providers: () => get<{ providers: string[]; count: number }>("/v1/models"),
  roles: () => get<{ roles: Record<string, string> }>("/v1/models/roles"),
};
