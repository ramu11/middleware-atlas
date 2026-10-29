const API_BASE = (process.env.NEXT_PUBLIC_ATLAS_API_BASE_URL ?? "http://127.0.0.1:8080").replace(/\/$/, "");

export type AtlasCase = {
  id: string;
  name: string;
  display_id: string;
  created_at: string;
  updated_at: string;
};

export type AtlasArtifact = {
  id: string;
  case_id: string;
  filename: string;
  kind: string;
  product: string;
  media_type: string;
  size_bytes: number;
  sha256: string;
  uploaded_at: string;
};

type ApiErrorBody = { error?: string };

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, {
      ...init,
      cache: "no-store",
      headers: {
        ...(init?.body instanceof FormData ? {} : { "Content-Type": "application/json" }),
        ...init?.headers,
      },
    });
  } catch {
    throw new Error(`Cannot connect to Middleware Atlas API at ${API_BASE}. Start the Rust API and retry.`);
  }

  if (!response.ok) {
    let message = `API request failed (${response.status})`;
    try {
      const body = (await response.json()) as ApiErrorBody;
      if (body.error) message = body.error;
    } catch {
      // Keep the status-based message if the server didn't return JSON.
    }
    throw new Error(message);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function listCases(): Promise<AtlasCase[]> {
  return request<AtlasCase[]>("/api/v1/cases");
}

export function createCase(name: string): Promise<AtlasCase> {
  return request<AtlasCase>("/api/v1/cases", {
    method: "POST",
    body: JSON.stringify({ name }),
  });
}

export function getCase(caseId: string): Promise<AtlasCase> {
  return request<AtlasCase>(`/api/v1/cases/${encodeURIComponent(caseId)}`);
}

export function updateCase(caseId: string, name: string): Promise<AtlasCase> {
  return request<AtlasCase>(`/api/v1/cases/${encodeURIComponent(caseId)}`, {
    method: "PATCH",
    body: JSON.stringify({ name }),
  });
}

export function deleteCase(caseId: string): Promise<void> {
  return request<void>(`/api/v1/cases/${encodeURIComponent(caseId)}`, {
    method: "DELETE",
  });
}

export function listArtifacts(caseId: string): Promise<AtlasArtifact[]> {
  return request<AtlasArtifact[]>(`/api/v1/cases/${encodeURIComponent(caseId)}/artifacts`);
}

export function uploadArtifact(
  caseId: string,
  file: File,
  onProgress?: (percent: number) => void,
): Promise<AtlasArtifact> {
  const form = new FormData();
  form.append("file", file, file.name);

  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${API_BASE}/api/v1/cases/${encodeURIComponent(caseId)}/artifacts`);
    xhr.responseType = "text";
    xhr.upload.addEventListener("progress", (event) => {
      if (event.lengthComputable && onProgress) {
        onProgress(Math.min(100, Math.round((event.loaded / event.total) * 100)));
      }
    });
    xhr.addEventListener("load", () => {
      let payload: unknown;
      try {
        payload = JSON.parse(xhr.responseText) as unknown;
      } catch {
        reject(new Error(`API returned an invalid response (${xhr.status}).`));
        return;
      }
      if (xhr.status >= 200 && xhr.status < 300) {
        onProgress?.(100);
        resolve(payload as AtlasArtifact);
        return;
      }
      const body = payload as ApiErrorBody;
      reject(new Error(body?.error ?? `Upload failed (${xhr.status}).`));
    });
    xhr.addEventListener("error", () => {
      reject(new Error(`Cannot connect to Middleware Atlas API at ${API_BASE}. Start the Rust API and retry.`));
    });
    xhr.addEventListener("abort", () => reject(new Error("Upload was cancelled.")));
    onProgress?.(0);
    xhr.send(form);
  });
}

export function deleteArtifact(caseId: string, artifactId: string): Promise<void> {
  return request<void>(
    `/api/v1/cases/${encodeURIComponent(caseId)}/artifacts/${encodeURIComponent(artifactId)}`,
    { method: "DELETE" },
  );
}

export function apiBaseUrl(): string {
  return API_BASE;
}


export type PreparedDiagnosticFile = {
  name: string;
  relative_path: string;
  purpose: string;
};

export type PreparedDiagnosticPlugin = {
  tool_run_id: string;
  job_id?: string | null;
  artifact_sha256?: string;
  tool_id: string;
  tool_version: string;
  status: string;
  artifact_id: string;
  artifact_filename: string;
  summary: string;
  workspace_relative_path: string;
  files: PreparedDiagnosticFile[];
  warnings: string[];
  llm_analysis?: string | null;
};

export type DiagnosticPreparationResponse = {
  case_id: string;
  status: string;
  message: string;
  preparations: PreparedDiagnosticPlugin[];
};

export type DiagnosticJobStatus = {
  id: string;
  case_id: string;
  operation: string;
  status: "queued" | "running" | "succeeded" | "failed" | "cancelled";
  progress: number; // Phase checkpoint from the worker/container, not time remaining.
  stage: string;
  cancel_requested: boolean;
  error?: string | null;
  result?: DiagnosticPreparationResponse | null;
  created_at: string;
  updated_at: string;
  started_at?: string | null;
  finished_at?: string | null;
};

function emitDiagnosticJob(job: DiagnosticJobStatus): void {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent("atlas:diagnostic-job-update", { detail: job }));
  }
}

export async function getDiagnosticJob(jobId: string): Promise<DiagnosticJobStatus> {
  return request<DiagnosticJobStatus>(`/api/v1/diagnostic-jobs/${encodeURIComponent(jobId)}`);
}

export async function cancelDiagnosticJob(jobId: string): Promise<DiagnosticJobStatus> {
  const job = await request<DiagnosticJobStatus>(`/api/v1/diagnostic-jobs/${encodeURIComponent(jobId)}/cancel`, { method: "POST" });
  emitDiagnosticJob(job);
  return job;
}

async function waitForDiagnosticJob(jobId: string): Promise<DiagnosticPreparationResponse> {
  const deadline = Date.now() + 24 * 60 * 60 * 1000;
  for (;;) {
    if (Date.now() >= deadline) throw new Error(`Diagnostic job ${jobId} is still running after 24 hours. Its durable status remains available from the job endpoint.`);
    const job = await getDiagnosticJob(jobId);
    emitDiagnosticJob(job);
    if (job.status === "failed") {
      if (job.result) return job.result;
      throw new Error(job.error || `Diagnostic job ${job.id} failed. Inspect the MAT job result and local container log.`);
    }
    if (job.status === "cancelled") throw new Error(`Diagnostic job ${job.id} was cancelled.`);
    if (job.status === "succeeded") {
      if (!job.result) throw new Error(`Diagnostic job ${job.id} finished without a result payload.`);
      return job.result;
    }
    await new Promise<void>((resolve) => window.setTimeout(resolve, 900));
  }
}

export function prepareDiagnostics(
  caseId: string,
  problemDescription: string,
  artifactIds: string[],
): Promise<DiagnosticPreparationResponse> {
  return request<DiagnosticPreparationResponse>(
    `/api/v1/cases/${encodeURIComponent(caseId)}/diagnostics/prepare`,
    {
      method: "POST",
      body: JSON.stringify({ problem_description: problemDescription, artifact_ids: artifactIds }),
    },
  ).then(async (initial) => {
    let merged = initial;
    for (const queued of initial.preparations.filter((item) => item.job_id)) {
      emitDiagnosticJob({
        id: queued.job_id!, case_id: initial.case_id, operation: "analyze_heap_dump",
        status: "queued", progress: 0, stage: "Queued for MAT analysis", cancel_requested: false,
        created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
      });
      const completed = await waitForDiagnosticJob(queued.job_id!);
      const finalPlugin = completed.preparations.find((item) => item.job_id === queued.job_id)
        ?? completed.preparations[0];
      if (finalPlugin) {
        merged = {
          ...merged,
          status: finalPlugin.status,
          message: completed.message,
          preparations: merged.preparations.map((item) => item.job_id === queued.job_id ? finalPlugin : item),
        };
      }
    }
    return merged;
  });
}

export type HeapOperationName =
  | "analyze_heap_dump"
  | "generate_leak_suspects"
  | "generate_heap_overview"
  | "get_histogram"
  | "get_dominator_tree"
  | "find_paths_to_gc_roots"
  | "compare_heap_dumps";

export type HeapOperationOptions = {
  problem_description?: string;
  artifact_id?: string;
  class_filter?: string | null;
  top_n?: number;
  object_id?: string;
  baseline_id?: string;
};

export function runHeapOperation(
  caseId: string,
  operation: HeapOperationName,
  options: HeapOperationOptions = {},
): Promise<DiagnosticPreparationResponse> {
  return request<DiagnosticJobStatus>(
    `/api/v1/cases/${encodeURIComponent(caseId)}/diagnostics/heap/${encodeURIComponent(operation)}`,
    { method: "POST", body: JSON.stringify(options) },
  ).then(async (job) => {
    emitDiagnosticJob(job);
    return waitForDiagnosticJob(job.id);
  });
}

// These named operations are intended for the agent/tool adapter to call.
export function analyze_heap_dump(case_id: string, options: HeapOperationOptions = {}) {
  return runHeapOperation(case_id, "analyze_heap_dump", options);
}
export function generate_leak_suspects(case_id: string, options: HeapOperationOptions = {}) {
  return runHeapOperation(case_id, "generate_leak_suspects", options);
}
export function generate_heap_overview(case_id: string, options: HeapOperationOptions = {}) {
  return runHeapOperation(case_id, "generate_heap_overview", options);
}
export function get_histogram(case_id: string, class_filter: string | null = null, options: Omit<HeapOperationOptions, "class_filter"> = {}) {
  return runHeapOperation(case_id, "get_histogram", { ...options, class_filter });
}
export function get_dominator_tree(case_id: string, top_n = 50, options: Omit<HeapOperationOptions, "top_n"> = {}) {
  return runHeapOperation(case_id, "get_dominator_tree", { ...options, top_n });
}
export function find_paths_to_gc_roots(case_id: string, object_id: string, options: Omit<HeapOperationOptions, "object_id"> = {}) {
  return runHeapOperation(case_id, "find_paths_to_gc_roots", { ...options, object_id });
}
export function compare_heap_dumps(case_id: string, baseline_id: string, options: Omit<HeapOperationOptions, "baseline_id"> = {}) {
  return runHeapOperation(case_id, "compare_heap_dumps", { ...options, baseline_id });
}


export type AgentMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  provider_message_id?: string | null;
  created_at: string;
};

export type AgentChatResponse = {
  incident_ref: string;
  conversation_active: boolean;
  needs_more_info: boolean;
  evidence_attached: boolean;
  messages: AgentMessage[];
};

export function sendAgentChat(caseId: string, message: string): Promise<AgentChatResponse> {
  return request<AgentChatResponse>(`/api/v1/cases/${encodeURIComponent(caseId)}/agent/chat`, {
    method: "POST",
    body: JSON.stringify({ message }),
  });
}

export function listAgentMessages(caseId: string): Promise<AgentMessage[]> {
  return request<AgentMessage[]>(`/api/v1/cases/${encodeURIComponent(caseId)}/agent/messages`);
}
