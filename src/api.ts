// Typed wrappers around the Tauri commands in src-tauri/src/lib.rs.
// Field names mirror the Rust structs (serde camelCase).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Tokens = { input: number; output: number; cacheRead: number; cacheCreation: number };

export type ProjectSummary = {
  id: number;
  name: string;
  path: string | null;
  location: string;
  sessionCount: number;
  promptCount: number;
  firstActivity: string | null;
  lastActivity: string | null;
  tokens: Tokens;
  estCost: number | null;
  sources: string[];
  latestScore: number | null;
};

export type SessionSummary = {
  id: number;
  projectId: number;
  source: string;
  location: string;
  externalId: string;
  title: string | null;
  firstPrompt: string | null;
  cwd: string | null;
  gitBranch: string | null;
  cliVersion: string | null;
  startedAt: string | null;
  endedAt: string | null;
  models: string | null;
  promptCount: number;
  messageCount: number;
  toolCallCount: number;
  errorCount: number;
  tokens: Tokens;
  estCost: number | null;
  filePath: string | null;
  latestScore: number | null;
};

export type MessageKind = "prompt" | "command" | "response" | "tool_call" | "tool_result" | "system";

export type MessageRow = {
  id: number;
  uuid: string;
  kind: MessageKind;
  timestamp: string | null;
  model: string | null;
  text: string;
  toolName: string | null;
  toolInput: string | null;
  isError: boolean;
  isSidechain: boolean;
  truncated: boolean;
};

export type SessionDetail = { session: SessionSummary; projectName: string; messages: MessageRow[] };

export type SearchHit = {
  messageId: number;
  sessionId: number;
  projectId: number;
  projectName: string;
  sessionTitle: string | null;
  kind: MessageKind;
  snippet: string;
  timestamp: string | null;
};

export type Criterion = {
  id: number | null;
  key: string;
  name: string;
  description: string;
  weight: number;
  enabled: boolean;
};

export type Scope = "prompt" | "session" | "project" | "global" | "draft";
export type ContextMode = "prompts" | "condensed" | "full";

export type Mistake = {
  title?: string;
  severity?: "high" | "medium" | "low" | string;
  refs?: string[];
  quote?: string;
  why?: string;
  impact?: string;
  fix?: string;
};
export type Rewrite = { ref?: string; original?: string; improved?: string; explanation?: string };
export type CriterionScore = { key?: string; score?: number; comment?: string };

export type AnalysisResult = {
  overall_score?: number;
  summary?: string;
  criteria?: CriterionScore[];
  strengths?: string[];
  mistakes?: Mistake[];
  rewrites?: Rewrite[];
  patterns?: string[];
  tips?: string[];
};

export type RefEntry = { ref: string; messageId?: number; sessionId?: number };

export type AnalysisRow = {
  id: number;
  scope: Scope;
  targetId: number | null;
  targetLabel: string | null;
  model: string;
  contextMode: ContextMode;
  status: "running" | "done" | "error";
  createdAt: string;
  finishedAt: string | null;
  overallScore: number | null;
  result: AnalysisResult | null;
  refMap: RefEntry[] | null;
  inputText: string | null;
  rawOutput: string | null;
  error: string | null;
  costUsd: number | null;
  durationMs: number | null;
  truncated: boolean;
};

export type PromptScore = { messageId: number; score: number; analysisId: number };

export type AnalysisRequest = {
  scope: Scope;
  targetId: number | null;
  mode: ContextMode;
  model: string;
  draftText?: string | null;
  draftContext?: string | null;
};

export type BuiltContext = { chars: number; estTokens: number; promptCount: number; truncated: boolean; notes: string[] };

export type SourceKind = "claude" | "codex";
export type SourceConfig = { id: string; kind: SourceKind; location: string; path: string; enabled: boolean; auto: boolean };

export type AppSettings = {
  sources: SourceConfig[];
  sourcesInitialized: boolean;
  claudePath: string | null;
  models: string[];
  defaultModel: string;
  defaultMode: ContextMode;
  maxContextChars: number;
  redact: boolean;
  pollSeconds: number;
  timeoutMinutes: number;
};

export type IngestReport = {
  filesScanned: number;
  filesImported: number;
  sessionsUpdated: number;
  sourcesSkipped: string[];
  errors: string[];
  finishedAt: string;
};
export type IngestStatus = { running: boolean; lastReport: IngestReport | null };

export type RetentionStatus = {
  settingsPath: string;
  exists: boolean;
  cleanupPeriodDays: number | null;
  effectiveDays: number;
  atRisk: boolean;
};

export type Stats = {
  projects: number;
  sessions: number;
  prompts: number;
  toolCalls: number;
  toolErrors: number;
  tokens: Tokens;
  estCost: number;
  analyses: number;
  byDay: { day: string; sessions: number; prompts: number; estCost: number }[];
  byModel: { model: string; responses: number; outputTokens: number; estCost: number | null }[];
  byProject: { projectId: number; name: string; sessions: number; prompts: number; outputTokens: number; estCost: number }[];
  bySource: { source: string; location: string; sessions: number; prompts: number }[];
};

export type AppInfo = { version: string; dbPath: string; workdir: string };
export type AnalysisEvent = { id: number; status: string; scope: Scope; targetId: number | null };

export const api = {
  rescan: (force = false) => invoke<IngestReport>("rescan", { force }),
  ingestStatus: () => invoke<IngestStatus>("ingest_status"),
  detectSources: () => invoke<SourceConfig[]>("detect_sources"),
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<AppSettings>("save_settings", { settings }),
  claudeCliStatus: () => invoke<string>("claude_cli_status"),
  retentionStatus: () => invoke<RetentionStatus>("retention_status"),
  fixRetention: () => invoke<RetentionStatus>("fix_retention"),
  appInfo: () => invoke<AppInfo>("app_info"),

  listProjects: () => invoke<ProjectSummary[]>("list_projects"),
  getProject: (id: number) => invoke<ProjectSummary | null>("get_project", { id }),
  renameProject: (id: number, name: string) => invoke<void>("rename_project", { id, name }),
  mergeProjects: (source: number, target: number) => invoke<void>("merge_projects", { source, target }),
  listSessions: (projectId: number) => invoke<SessionSummary[]>("list_sessions", { projectId }),
  getSession: (id: number) => invoke<SessionDetail>("get_session", { id }),
  getMessage: (id: number) => invoke<MessageRow>("get_message", { id }),
  messageSession: (id: number) => invoke<number>("message_session", { id }),
  promptScores: (sessionId: number) => invoke<PromptScore[]>("prompt_scores", { sessionId }),
  search: (query: string, kinds: string[], projectId: number | null) =>
    invoke<SearchHit[]>("search", { query, kinds, projectId }),
  stats: (days = 90) => invoke<Stats>("stats", { days }),

  listCriteria: () => invoke<Criterion[]>("list_criteria"),
  saveCriteria: (criteria: Criterion[]) => invoke<Criterion[]>("save_criteria", { criteria }),
  resetCriteria: () => invoke<Criterion[]>("reset_criteria"),

  previewAnalysis: (request: AnalysisRequest) => invoke<BuiltContext>("preview_analysis", { request }),
  previewAnalysisText: (request: AnalysisRequest) => invoke<string>("preview_analysis_text", { request }),
  startAnalysis: (request: AnalysisRequest) => invoke<number>("start_analysis", { request }),
  cancelAnalysis: (id: number) => invoke<void>("cancel_analysis", { id }),
  listAnalyses: (scope: Scope | null, targetId: number | null) =>
    invoke<AnalysisRow[]>("list_analyses", { scope, targetId }),
  sessionPromptAnalyses: (sessionId: number) => invoke<AnalysisRow[]>("session_prompt_analyses", { sessionId }),
  getAnalysis: (id: number) => invoke<AnalysisRow>("get_analysis", { id }),
  deleteAnalysis: (id: number) => invoke<void>("delete_analysis", { id }),

  exportSession: (id: number, format: "md" | "json", path: string) => invoke<void>("export_session", { id, format, path }),
  exportAnalysis: (id: number, format: "md" | "json", path: string) => invoke<void>("export_analysis", { id, format, path }),
};

export function onEvent<T>(name: "ingest-status" | "analysis-updated", handler: (payload: T) => void): () => void {
  let unlisten: UnlistenFn | null = null;
  let disposed = false;
  listen<T>(name, (e) => handler(e.payload)).then((u) => {
    if (disposed) u();
    else unlisten = u;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
