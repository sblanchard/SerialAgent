import { ref, computed, readonly } from "vue";
import { api, ApiError } from "@/api/client";
import type {
  ImportSource,
  ImportOptions,
  ImportPreviewResponse,
  ImportApplyResponseV2,
  MergeStrategy,
  SshAuth,
} from "@/api/client";

// ── State machine ───────────────────────────────────────────────────

export type WizardPhase =
  | "idle"
  | "validating"
  | "scanned"
  | "previewing"
  | "applying"
  | "done"
  | "error";

export type SourceTab = "local" | "ssh";
export type Preset = "minimal" | "full" | "everything" | "custom";

export interface ImportError {
  friendly: string;
  detail: string;
  status?: number;
}

// ── Import limits (shown to user) ───────────────────────────────────

export const IMPORT_LIMITS = {
  maxTgzBytes: 200 * 1024 * 1024,
  maxExtractedBytes: 500 * 1024 * 1024,
  maxFileCount: 50_000,
  maxEntriesTotal: 100_000,
  maxPathDepth: 64,
  rejectedTypes: ["symlinks", "hardlinks", "devices", "FIFOs"],
} as const;

// ── Composable ──────────────────────────────────────────────────────

export function useOpenClawImport() {
  // Phase
  const phase = ref<WizardPhase>("idle");

  // Source
  const sourceTab = ref<SourceTab>("local");
  const localPath = ref("/home/user/.openclaw");
  const sshHost = ref("");
  const sshUser = ref("");
  const sshPort = ref("");
  const sshRemotePath = ref("~/.openclaw");
  const sshAuthMethod = ref<"agent" | "keyfile" | "password">("agent");
  const sshKeyPath = ref("~/.ssh/id_ed25519");
  const sshPassword = ref("");
  const sshTesting = ref(false);
  const sshTestResult = ref<{ ok: boolean; message: string } | null>(null);

  // Preset
  const preset = ref<Preset>("full");
  const customWorkspaces = ref(true);
  const customSessions = ref(true);
  const customModels = ref(false);
  const customAuth = ref(false);

  // Results
  const previewData = ref<ImportPreviewResponse | null>(null);
  const applyResult = ref<ImportApplyResponseV2 | null>(null);
  const mergeStrategy = ref<MergeStrategy>("merge_safe");

  // Errors
  const error = ref<ImportError | null>(null);

  // ── Computed ────────────────────────────────────────────────────

  const options = computed<ImportOptions>(() => {
    switch (preset.value) {
      case "minimal":
        return { include_workspaces: true, include_sessions: true };
      case "full":
        return {
          include_workspaces: true,
          include_sessions: true,
          include_models: true,
        };
      case "everything":
        return {
          include_workspaces: true,
          include_sessions: true,
          include_models: true,
          include_auth_profiles: true,
        };
      case "custom":
        return {
          include_workspaces: customWorkspaces.value,
          include_sessions: customSessions.value,
          include_models: customModels.value,
          include_auth_profiles: customAuth.value,
        };
    }
  });

  const canScan = computed(() => {
    if (phase.value === "validating") return false;
    if (sourceTab.value === "local") return !!localPath.value.trim();
    return !!sshHost.value.trim();
  });

  const isAuthEnabled = computed(
    () =>
      preset.value === "everything" ||
      (preset.value === "custom" && customAuth.value)
  );

  // ── Build source ───────────────────────────────────────────────

  function buildSource(): ImportSource {
    if (sourceTab.value === "local") {
      return { local: { path: localPath.value } };
    }
    const auth: SshAuth =
      sshAuthMethod.value === "keyfile"
        ? { key_file: { key_path: sshKeyPath.value } }
        : sshAuthMethod.value === "password"
          ? { password: { password: sshPassword.value } }
          : "agent";
    return {
      ssh: {
        host: sshHost.value,
        user: sshUser.value || undefined,
        port: sshPort.value ? parseInt(sshPort.value) : undefined,
        remote_path: sshRemotePath.value,
        auth,
      },
    };
  }

  // ── Actions ────────────────────────────────────────────────────

  async function testSsh() {
    sshTesting.value = true;
    sshTestResult.value = null;
    try {
      const auth: SshAuth =
        sshAuthMethod.value === "keyfile"
          ? { key_file: { key_path: sshKeyPath.value } }
          : sshAuthMethod.value === "password"
            ? { password: { password: sshPassword.value } }
            : "agent";
      const res = await api.testSsh(
        sshHost.value,
        sshUser.value || undefined,
        sshPort.value ? parseInt(sshPort.value) : undefined,
        auth
      );
      sshTestResult.value = {
        ok: res.ok,
        message: res.ok
          ? "Connection successful"
          : res.stderr || res.error || "Failed",
      };
    } catch (e: unknown) {
      const err = e instanceof ApiError ? e : new Error(String(e));
      sshTestResult.value = { ok: false, message: err.message };
    } finally {
      sshTesting.value = false;
    }
  }

  async function preview() {
    phase.value = "validating";
    error.value = null;
    previewData.value = null;
    try {
      previewData.value = await api.importPreview({
        source: buildSource(),
        options: options.value,
      });
      phase.value = "scanned";
    } catch (e: unknown) {
      setError(e);
    }
  }

  function confirmPreview() {
    phase.value = "previewing";
  }

  async function apply() {
    if (!previewData.value) return;
    phase.value = "applying";
    error.value = null;
    try {
      applyResult.value = await api.importApply({
        staging_id: previewData.value.staging_id,
        merge_strategy: mergeStrategy.value,
        options: options.value,
      });
      phase.value = "done";
    } catch (e: unknown) {
      setError(e);
    }
  }

  function reset() {
    phase.value = "idle";
    previewData.value = null;
    applyResult.value = null;
    error.value = null;
  }

  function goBack() {
    switch (phase.value) {
      case "scanned":
      case "previewing":
        phase.value = "scanned";
        break;
      case "error":
        if (previewData.value) {
          phase.value = "scanned";
        } else {
          phase.value = "idle";
        }
        break;
      default:
        phase.value = "idle";
    }
    error.value = null;
  }

  function dismissError() {
    if (previewData.value) {
      phase.value = "scanned";
    } else {
      phase.value = "idle";
    }
    error.value = null;
  }

  // ── Helpers ────────────────────────────────────────────────────

  function setError(e: unknown) {
    if (e instanceof ApiError) {
      error.value = {
        friendly: e.friendly,
        detail: e.detail,
        status: e.status,
      };
    } else {
      const msg = e instanceof Error ? e.message : String(e);
      error.value = { friendly: msg, detail: msg };
    }
    phase.value = "error";
  }

  // ── Expose ─────────────────────────────────────────────────────

  return {
    // State
    phase: readonly(phase),
    sourceTab,
    localPath,
    sshHost,
    sshUser,
    sshPort,
    sshRemotePath,
    sshAuthMethod,
    sshKeyPath,
    sshPassword,
    sshTesting: readonly(sshTesting),
    sshTestResult: readonly(sshTestResult),
    preset,
    customWorkspaces,
    customSessions,
    customModels,
    customAuth,
    options,
    canScan,
    isAuthEnabled,
    previewData: readonly(previewData),
    applyResult: readonly(applyResult),
    mergeStrategy,
    error: readonly(error),

    // Actions
    testSsh,
    preview,
    confirmPreview,
    apply,
    reset,
    goBack,
    dismissError,
  };
}
