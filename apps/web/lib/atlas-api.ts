const API_BASE = (process.env.NEXT_PUBLIC_ATLAS_API_BASE_URL ?? "http://127.0.0.1:8080").replace(/\/$/, "");

export type AtlasCase = {
  id: string;
  name: string;
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

export function listArtifacts(caseId: string): Promise<AtlasArtifact[]> {
  return request<AtlasArtifact[]>(`/api/v1/cases/${encodeURIComponent(caseId)}/artifacts`);
}

export function uploadArtifact(caseId: string, file: File): Promise<AtlasArtifact> {
  const form = new FormData();
  form.append("file", file, file.name);
  return request<AtlasArtifact>(`/api/v1/cases/${encodeURIComponent(caseId)}/artifacts`, {
    method: "POST",
    body: form,
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
