// Typed API client wrappers for the SerialAgent gateway.

const BASE = "";

// ── Structured API error with friendly messages ─────────────────────

export class ApiError extends Error {
  status: number;
  detail: string;
  friendly: string;

  constructor(method: string, path: string, status: number, body: string) {
    const detail = extractDetail(body);
    const friendly = mapFriendlyMessage(status, detail);
    super(friendly);
    this.name = "ApiError";
    this.status = status;
    this.detail = detail;
    this.friendly = friendly;
  }
}

function extractDetail(body: string): string {
  try {
    const json = JSON.parse(body);
    return json.error || json.message || body;
  } catch {
    return body;
  }
}

function mapFriendlyMessage(status: number, detail: string): string {
  const d = detail.toLowerCase();
  if (status === 413) return "Archive exceeds size limits";
  if (status === 401) return "Admin authentication required";
  if (d.includes("traversal") || d.includes("parent") || d.includes("absolute"))
    return "Unsafe paths detected in archive";
  if (d.includes("non-utf8") || d.includes("non-utf-8"))
    return "Unsupported filename encoding in archive";
  if (d.includes("duplicate"))
    return "Conflicting duplicate entries in archive";
  if (d.includes("symlink") || d.includes("link") || d.includes("device"))
    return "Symlinks, hardlinks, or devices not allowed";
  if (d.includes("size limit"))
    return "Archive exceeds size limits";
  if (d.includes("ssh") || d.includes("connect"))
    return "SSH connection failed";
  if (d.includes("staging") && d.includes("not found"))
    return "Staging data not found (may have expired)";
  if (status === 502) return "Remote connection failed";
  if (status >= 500) return "Server error — please try again";
  return detail;
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new ApiError("GET", path, res.status, body);
  }
  return res.json();
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const respBody = await res.text().catch(() => "");
    throw new ApiError("POST", path, res.status, respBody);
  }
  return res.json();
}

async function put<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const respBody = await res.text().catch(() => "");
    throw new ApiError("PUT", path, res.status, respBody);
  }
  return res.json();
}

async function del<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: "DELETE" });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new ApiError("DELETE", path, res.status, body);
  }
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

// ── Staging management types ────────────────────────────────────────

export type StagingEntry = {
  id: string;
  created_at: string;
  age_secs: number;
  size_bytes: number;
  has_extracted: boolean;
};

export type StagingListResponse = {
  entries: StagingEntry[];
  count: number;
};

export type DeleteStagingResponse = {
  deleted: boolean;
};

// ── Run types ───────────────────────────────────────────────────────

export type RunStatus = "queued" | "running" | "completed" | "failed" | "stopped";

export type NodeKind = "llm_request" | "tool_call";

export type RunNode = {
  node_id: number;
  kind: NodeKind;
  name: string;
  status: RunStatus;
  started_at: string;
  ended_at?: string;
  duration_ms?: number;
  input_preview?: string;
  output_preview?: string;
  is_error: boolean;
  input_tokens: number;
  output_tokens: number;
};

export type RunListItem = {
  run_id: string;
  session_key: string;
  session_id: string;
  status: RunStatus;
  agent_id?: string;
  model?: string;
  started_at: string;
  ended_at?: string;
  duration_ms?: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  input_preview?: string;
  output_preview?: string;
  error?: string;
  node_count: number;
  loop_count: number;
};

export type RunDetail = RunListItem & {
  nodes: RunNode[];
};

export type RunListResponse = {
  runs: RunListItem[];
  total: number;
  limit: number;
  offset: number;
};

export type RunNodesResponse = {
  run_id: string;
  nodes: RunNode[];
  count: number;
};

export type RunListParams = {
  status?: RunStatus;
  session_key?: string;
  agent_id?: string;
  limit?: number;
  offset?: number;
};

// ── Schedule types ───────────────────────────────────────────────────

export type ScheduleStatus = "active" | "paused" | "error";
export type MissedPolicy = "skip" | "run_once" | "catch_up";
export type DigestMode = "full" | "changes_only";

export type DeliveryTarget =
  | { kind: "in_app" }
  | { kind: "webhook"; url: string };

export type FetchConfig = {
  timeout_ms: number;
  user_agent: string;
  max_size_bytes: number;
};

export type SourceState = {
  last_fetched_at?: string;
  last_content_hash?: string;
  last_http_status?: number;
  last_error?: string;
};

export type Schedule = {
  id: string;
  name: string;
  cron: string;
  timezone: string;
  enabled: boolean;
  agent_id: string;
  prompt_template: string;
  sources: string[];
  delivery_targets: DeliveryTarget[];
  created_at: string;
  updated_at: string;
  last_run_id?: string;
  last_run_at?: string;
  next_run_at?: string;
  /** Computed by the backend from enabled + consecutive_failures. */
  status: ScheduleStatus;
  missed_policy: MissedPolicy;
  max_concurrency: number;
  timeout_ms?: number;
  digest_mode: DigestMode;
  fetch_config: FetchConfig;
  source_states: Record<string, SourceState>;
  last_error?: string;
  last_error_at?: string;
  consecutive_failures: number;
};

export type ScheduleListResponse = {
  schedules: Schedule[];
  count: number;
};

export type ScheduleDetailResponse = {
  schedule: Schedule;
  next_occurrences: string[];
};

export type CreateScheduleRequest = {
  name: string;
  cron: string;
  timezone?: string;
  enabled?: boolean;
  agent_id?: string;
  prompt_template: string;
  sources?: string[];
  delivery_targets?: DeliveryTarget[];
  missed_policy?: MissedPolicy;
  max_concurrency?: number;
  timeout_ms?: number;
  digest_mode?: DigestMode;
  fetch_config?: Partial<FetchConfig>;
};

export type UpdateScheduleRequest = {
  name?: string;
  cron?: string;
  timezone?: string;
  enabled?: boolean;
  agent_id?: string;
  prompt_template?: string;
  sources?: string[];
  delivery_targets?: DeliveryTarget[];
  missed_policy?: MissedPolicy;
  max_concurrency?: number;
  timeout_ms?: number | null;
  digest_mode?: DigestMode;
  fetch_config?: Partial<FetchConfig>;
};

// ── Delivery types ──────────────────────────────────────────────────

export type Delivery = {
  id: string;
  schedule_id?: string;
  schedule_name?: string;
  run_id?: string;
  created_at: string;
  title: string;
  body: string;
  sources: string[];
  read: boolean;
  metadata: unknown;
};

export type DeliveryListResponse = {
  deliveries: Delivery[];
  total: number;
  unread: number;
};

// ── Skill engine types ──────────────────────────────────────────────

export type DangerLevel = "safe" | "network" | "filesystem" | "execution";

export type SkillEngineSpec = {
  name: string;
  title: string;
  description: string;
  args_schema: unknown;
  returns_schema: unknown;
  danger_level: DangerLevel;
};

export type SkillEngineListResponse = {
  skills: SkillEngineSpec[];
  count: number;
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
  listStaging: () =>
    get<StagingListResponse>("/v1/import/openclaw/staging"),
  deleteStaging: (id: string) =>
    del<DeleteStagingResponse>(`/v1/import/openclaw/staging/${encodeURIComponent(id)}`),

  // Runs
  getRuns: (params?: RunListParams) => {
    const q = new URLSearchParams();
    if (params?.status) q.set("status", params.status);
    if (params?.session_key) q.set("session_key", params.session_key);
    if (params?.agent_id) q.set("agent_id", params.agent_id);
    if (params?.limit) q.set("limit", String(params.limit));
    if (params?.offset) q.set("offset", String(params.offset));
    const qs = q.toString();
    return get<RunListResponse>(`/v1/runs${qs ? "?" + qs : ""}`);
  },
  getRun: (runId: string) =>
    get<RunDetail>(`/v1/runs/${encodeURIComponent(runId)}`),
  getRunNodes: (runId: string) =>
    get<RunNodesResponse>(`/v1/runs/${encodeURIComponent(runId)}/nodes`),

  // Schedules
  getSchedules: () => get<ScheduleListResponse>("/v1/schedules"),
  getSchedule: (id: string) =>
    get<ScheduleDetailResponse>(`/v1/schedules/${encodeURIComponent(id)}`),
  createSchedule: (req: CreateScheduleRequest) =>
    post<{ schedule: Schedule }>("/v1/schedules", req),
  updateSchedule: (id: string, req: UpdateScheduleRequest) =>
    put<{ schedule: Schedule }>(`/v1/schedules/${encodeURIComponent(id)}`, req),
  deleteSchedule: (id: string) =>
    del<{ deleted: boolean }>(`/v1/schedules/${encodeURIComponent(id)}`),
  runScheduleNow: (id: string) =>
    post<{ run_id: string; schedule_id: string }>(`/v1/schedules/${encodeURIComponent(id)}/run-now`, {}),

  // Deliveries (inbox)
  getDeliveries: (limit = 25, offset = 0) =>
    get<DeliveryListResponse>(`/v1/deliveries?limit=${limit}&offset=${offset}`),
  getDelivery: (id: string) =>
    get<{ delivery: Delivery }>(`/v1/deliveries/${encodeURIComponent(id)}`),
  markDeliveryRead: (id: string) =>
    post<{ ok: boolean }>(`/v1/deliveries/${encodeURIComponent(id)}/read`, {}),

  // Skill engine
  getSkillEngine: () => get<SkillEngineListResponse>("/v1/skill-engine"),

  // Provider listing
  providers: () => get<{ providers: string[]; count: number }>("/v1/models"),
  roles: () => get<{ roles: Record<string, string> }>("/v1/models/roles"),
};
