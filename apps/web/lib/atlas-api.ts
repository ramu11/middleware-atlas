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
