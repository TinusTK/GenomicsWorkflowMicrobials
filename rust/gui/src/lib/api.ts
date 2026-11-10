// Talks to the app's local server (`gwm serve`). In the desktop window the
// server runs beside it on port 8642; in a browser it is the same origin.

const inDesktop = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
export const API_BASE = inDesktop ? 'http://127.0.0.1:8642' : '';

export class ApiError extends Error {}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(API_BASE + path, {
      method,
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError('The app\'s background service is not answering. It may still be starting.');
  }
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(data.error ?? `Request failed (${res.status})`);
  return data as T;
}

export const get = <T>(path: string) => call<T>('GET', path);
export const post = <T>(path: string, body: unknown = {}) => call<T>('POST', path, body);
export const del = <T>(path: string) => call<T>('DELETE', path);

export function fileUrl(runId: string, path: string, download = false) {
  return `${API_BASE}/api/runs/${encodeURIComponent(runId)}/file?path=${encodeURIComponent(path)}${download ? '&download=1' : ''}`;
}

export async function fileText(runId: string, path: string): Promise<string | null> {
  const res = await fetch(fileUrl(runId, path));
  return res.ok ? res.text() : null;
}

// ---------------------------------------------------------------- types

export interface Usage {
  cpus: number;
  cpu_percent: number;
  cpu_ghz: number | null;
  memory_gb: number;
  memory_used_gb: number;
  disk_free_gb: number | null;
  disk_total_gb: number | null;
}

export interface Hardware {
  cpus: number;
  memory_gb: number;
  memory_available_gb: number;
  wsl: boolean;
  os: string | null;
  docker: string | null;
  docker_error: string | null;
  apptainer: string | null;
  disk_free_gb: { work: number | null; databases: number | null };
  app_version: string;
  runs_folder: string;
  data_folder: string;
  work_folder: string;
  databases_folder: string;
}

export interface WorkflowSummary {
  id: string;
  name: string;
  description: string;
  builtin: boolean;
  versions: number[];
  latest: number;
}

export interface Step {
  tool: string;
  tool_version?: string;
  label?: string;
  inputs?: Record<string, string[]>;
  settings?: Record<string, unknown>;
  enabled?: boolean;
  publish?: string;
  [k: string]: unknown;
}

export interface Workflow {
  id: string;
  name: string;
  version: number;
  description: string;
  steps: Record<string, Step>;
  layout?: Record<string, { x: number; y: number }>;
  [k: string]: unknown;
}

export interface WorkflowView {
  workflow: Workflow;
  builtin: boolean;
  version: number;
  versions: number[];
  order: string[];
  runnable: Record<string, boolean> | null;
  error: string | null;
}

export interface SettingSpec {
  label?: string;
  type?: string;
  default?: unknown;
  options?: string[];
  auto?: { value: unknown; label: string };
  description?: string;
}

export interface Tool {
  id: string;
  name: string;
  version: string;
  category?: string;
  description?: string;
  container: string;
  scope: string;
  /** `collect`: takes every connected source (any number) whose type is in `accepts` */
  inputs: Record<string, { type: string; optional: boolean; description: string; collect?: boolean; accepts?: string[] }>;
  outputs: Record<string, { type: string; path: string; optional: boolean; description: string }>;
  settings: Record<string, SettingSpec>;
  databases: string[];
  source: string;
  variants: string[];
  homepage?: string;
  license?: string;
  citation?: string;
  /** where its tool.yml is */
  folder?: string;
  /** outputs switched off for the Run page (all show by default) */
  hidden_outputs: string[];
  /** not ready: its command is not written yet (workflows refuse to run it) */
  draft?: boolean;
  command?: string;
  resources?: { cpus?: number; memory_gb?: number };
  version_command?: string;
  /** the program the editor asks for its help text */
  program?: string;
  /** added from this GitHub repository */
  github?: { repo: string; branch?: string };
}

/** One step of a Galaxy workflow and what the import makes of it. */
export interface ImportStep {
  galaxy_step: number;
  name: string;
  galaxy_tool: string;
  galaxy_version: string;
  action: 'builtin' | 'new_version' | 'new_tool' | 'summary' | 'input' | 'left_out';
  step?: string;
  tool?: string;
  tool_version?: string;
  note: string;
}

export interface ImportResult {
  id: string;
  name: string;
  steps: ImportStep[];
  version?: number;
}

export interface Database {
  id: string;
  name: string;
  version: string;
  size_gb: number;
  description?: string;
  galaxy_note?: string;
  installed: boolean;
  path: string;
}

export interface Job {
  id: string;
  title: string;
  status: 'running' | 'done' | 'failed';
  log: string[];
  progress: [number, number] | null;
  /** what the work found (a tool's help text, what a trial made) */
  result?: unknown;
}

export interface Sample {
  sample: string;
  r1: string;
  r2: string | null;
}

export interface Found {
  samples: Sample[];
  unpaired: string[];
  r1: string;
  r2: string;
  mode: string;
}

export interface Warning {
  level: 'error' | 'warning' | 'info';
  step: string | null;
  message: string;
}

export interface RunSummary {
  id: string;
  name: string;
  workflow: { id: string; version: number; name: string };
  status: string;
  started: string;
  finished: string | null;
  n_samples: number;
  folder?: string;
}

export interface TaskState {
  status: string;
  started_at?: string;
  finished_at?: string;
  exit?: number;
}

export interface PlanStep {
  id: string;
  label: string;
  tool: string;
  version: string;
  per_sample: boolean;
  enabled: boolean;
  runnable: boolean;
  inputs: string[];
  tool_id?: string;
  /** results folder, `{sample}` = the sample */
  publish?: string;
  outputs?: Record<string, { path: string; optional: boolean }>;
  /** input -> its sources (`input` or `<step>.<output>`), first that runs wins */
  wiring?: Record<string, string[]>;
  /** inputs that use every source, not just the first that runs */
  collect?: string[];
}

export interface RunDetail extends RunSummary {
  input: string;
  samples: string[];
  steps: Record<string, { tasks: Record<string, TaskState> }>;
  steps_off: string[];
  error: string | null;
  results: string;
  plan_steps?: PlanStep[];
  files?: { path: string; size: number }[];
  engine?: { version: string; containers: string };
  limits?: { cpus: number; memory_gb: number; total_memory_gb: number };
}
