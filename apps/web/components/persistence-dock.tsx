"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { CheckCircle2, ChevronDown, ChevronUp, Database, FileText, RefreshCw, Trash2, WifiOff } from "lucide-react";
import { apiBaseUrl, deleteArtifact, listArtifacts, listCases, type AtlasArtifact, type AtlasCase } from "@/lib/atlas-api";

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unit = units[0];
  for (let index = 0; size >= 1024 && index < units.length - 1; index += 1) {
    size /= 1024;
    unit = units[index + 1];
  }
  return `${size.toFixed(size >= 10 ? 0 : 1)} ${unit}`;
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function PersistenceDock() {
  const [expanded, setExpanded] = useState(false);
  const [cases, setCases] = useState<AtlasCase[]>([]);
  const [selectedCaseId, setSelectedCaseId] = useState("");
  const [artifacts, setArtifacts] = useState<AtlasArtifact[]>([]);
  const [online, setOnline] = useState(false);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState("");

  const refreshGeneration = useRef(0);

  const refreshCases = useCallback(async (preferredId?: string) => {
    const generation = ++refreshGeneration.current;
    setLoading(true);
    try {
      const rows = await listCases();
      if (generation !== refreshGeneration.current) return;
      setCases(rows);
      setOnline(true);
      const stored = typeof window !== "undefined" ? window.localStorage.getItem("atlas.currentCaseId") ?? "" : "";
      const wanted = preferredId || stored;
      const selected = rows.some((row) => row.id === wanted) ? wanted : "";
      setSelectedCaseId(selected);
      // Browsing history must not silently make an old case the active upload target.
      // Persist the active ID only when it was already active or explicitly supplied.
      if (selected && typeof window !== "undefined") {
        if (preferredId || stored) window.localStorage.setItem("atlas.currentCaseId", selected);
        // On page reload, hydrate the form from the active persisted case before the next save.
        if (!preferredId) {
          const activeCase = rows.find((row) => row.id === selected);
          window.dispatchEvent(new CustomEvent("atlas:case-selected", { detail: { caseId: selected, name: activeCase?.name ?? "" } }));
        }
      }
      setMessage("");
    } catch (error) {
      if (generation !== refreshGeneration.current) return;
      setOnline(false);
      setMessage(error instanceof Error ? error.message : "Could not reach the local API.");
    } finally {
      if (generation === refreshGeneration.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Defer the initial refresh past this effect's synchronous phase to satisfy React Hooks lint.
    const initialRefreshTimer = window.setTimeout(() => {
      void refreshCases();
    }, 0);
    const onSaved = (event: Event) => {
      const detail = (event as CustomEvent<{ caseId?: string }>).detail;
      const caseId = detail?.caseId;
      void refreshCases(caseId);
      if (!caseId) return;
      void listArtifacts(caseId)
        .then((rows) => {
          // A slow refresh for an old case must not replace the currently selected case's artifacts.
          if (window.localStorage.getItem("atlas.currentCaseId") === caseId) setArtifacts(rows);
        })
        .catch((error) => {
          if (window.localStorage.getItem("atlas.currentCaseId") === caseId) {
            setMessage(error instanceof Error ? error.message : "Could not refresh artifacts.");
          }
        });
    };
    const onNewCase = () => {
      refreshGeneration.current += 1;
      setLoading(false);
      setSelectedCaseId("");
      setArtifacts([]);
      setMessage("New case selected. Use “Continue to case details” to save it.");
    };
    const onCaseSelected = (event: Event) => {
      const detail = (event as CustomEvent<{ caseId?: string }>).detail;
      if (detail?.caseId) {
        refreshGeneration.current += 1;
        setLoading(false);
        setArtifacts([]);
        setSelectedCaseId(detail.caseId);
      }
    };
    window.addEventListener("atlas:case-saved", onSaved);
    window.addEventListener("atlas:new-case", onNewCase);
    window.addEventListener("atlas:case-selected", onCaseSelected);
    return () => {
      // Invalidate outstanding requests so they cannot update state after unmount.
      refreshGeneration.current += 1;
      window.clearTimeout(initialRefreshTimer);
      window.removeEventListener("atlas:case-saved", onSaved);
      window.removeEventListener("atlas:new-case", onNewCase);
      window.removeEventListener("atlas:case-selected", onCaseSelected);
    };
  }, [refreshCases]);

  useEffect(() => {
    if (!selectedCaseId) return;
    let cancelled = false;
    void listArtifacts(selectedCaseId)
      .then((rows) => { if (!cancelled) setArtifacts(rows); })
      .catch((error) => { if (!cancelled) setMessage(error instanceof Error ? error.message : "Could not load artifacts."); });
    return () => { cancelled = true; };
  }, [selectedCaseId]);

  const selectedCase = cases.find((item) => item.id === selectedCaseId);

  async function removeArtifact(artifact: AtlasArtifact) {
    if (!selectedCaseId) return;
    const confirmed = window.confirm(`Remove “${artifact.filename}” from this case and delete its local file?`);
    if (!confirmed) return;
    try {
      await deleteArtifact(selectedCaseId, artifact.id);
      try {
        const storageKey = `atlas.uploadedFiles.${selectedCaseId}`;
        const raw = window.localStorage.getItem(storageKey);
        if (raw) {
          const uploaded = JSON.parse(raw) as Record<string, string>;
          for (const [fingerprint, savedId] of Object.entries(uploaded)) {
            if (savedId === artifact.id) delete uploaded[fingerprint];
          }
          window.localStorage.setItem(storageKey, JSON.stringify(uploaded));
        }
      } catch { /* Server-side deletion remains authoritative if local bookkeeping is unreadable. */ }
      setArtifacts((current) => current.filter((item) => item.id !== artifact.id));
      setMessage("Artifact deleted from local storage.");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Could not delete artifact.");
    }
  }

  const buttonStyle = {
    border: "1px solid #d7dce4",
    borderRadius: 999,
    background: "#ffffff",
    color: "#202838",
    padding: "11px 15px",
    boxShadow: "0 8px 30px rgba(20, 29, 48, .13)",
    display: "flex",
    alignItems: "center",
    gap: 8,
    cursor: "pointer",
    font: "600 13px/1.2 system-ui, sans-serif",
  } as const;

  return (
    <div style={{ position: "fixed", right: 22, bottom: 20, zIndex: 10000, color: "#202838", fontFamily: "system-ui, sans-serif" }}>
      {expanded && (
        <section aria-label="Persisted cases" style={{ width: "min(390px, calc(100vw - 32px))", maxHeight: "min(600px, calc(100vh - 110px))", overflow: "auto", marginBottom: 10, border: "1px solid #dce1e9", borderRadius: 18, background: "#fff", boxShadow: "0 18px 60px rgba(20, 29, 48, .19)" }}>
          <header style={{ padding: "16px 17px 13px", borderBottom: "1px solid #edf0f4", display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 12 }}>
            <div>
              <div style={{ display: "flex", alignItems: "center", gap: 8, fontWeight: 700, fontSize: 14 }}><Database size={17} /> Local persistence</div>
              <div style={{ marginTop: 4, color: "#687386", fontSize: 11 }}>SQLite metadata · local files</div>
            </div>
            <button type="button" aria-label="Refresh saved cases" onClick={() => { setLoading(true); void refreshCases(selectedCaseId); }} style={{ border: 0, background: "transparent", color: "#606c7e", cursor: "pointer", padding: 4 }}><RefreshCw size={16} /></button>
          </header>
          <div style={{ padding: 16 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 7, fontSize: 12, marginBottom: 10, color: online ? "#167356" : "#b45309" }}>
              {online ? <CheckCircle2 size={15} /> : <WifiOff size={15} />}
              {online ? "API connected" : "API unavailable"}
            </div>
            {!online && <p style={{ margin: "0 0 12px", color: "#6b7280", fontSize: 12 }}>Start `cargo run -p atlas-api`. Expected endpoint: {apiBaseUrl()}</p>}
            <label htmlFor="atlas-saved-case" style={{ display: "block", fontSize: 11, fontWeight: 700, color: "#667085", marginBottom: 6, textTransform: "uppercase", letterSpacing: ".06em" }}>Saved cases</label>
            <select id="atlas-saved-case" value={selectedCaseId} onChange={(event) => {
              const id = event.target.value;
              if (id !== selectedCaseId) setArtifacts([]);
              setSelectedCaseId(id);
              if (id) {
                window.localStorage.setItem("atlas.currentCaseId", id);
                const selected = cases.find((item) => item.id === id);
                window.dispatchEvent(new CustomEvent("atlas:case-selected", { detail: { caseId: id, name: selected?.name ?? "" } }));
              }
            }} style={{ width: "100%", border: "1px solid #d7dce4", background: "white", borderRadius: 9, padding: "10px 11px", color: "#202838", fontSize: 13 }}>
              <option value="">{cases.length ? "Select a saved case…" : "No saved cases yet"}</option>
              {cases.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
            </select>
            {selectedCase && <div style={{ fontSize: 11, color: "#7a8494", marginTop: 6 }}>Updated {formatDate(selectedCase.updated_at)}</div>}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 10, marginTop: 18, marginBottom: 9 }}>
              <div style={{ fontSize: 12, fontWeight: 700 }}>Artifacts <span style={{ color: "#7a8494", fontWeight: 500 }}>({artifacts.length})</span></div>
              <button type="button" onClick={() => {
                window.localStorage.removeItem("atlas.currentCaseId");
                window.dispatchEvent(new CustomEvent("atlas:new-case"));
              }} style={{ border: "1px solid #d7dce4", background: "#fff", borderRadius: 8, padding: "6px 9px", color: "#39465a", fontSize: 11, cursor: "pointer" }}>New case</button>
            </div>
            {loading && <div style={{ fontSize: 12, color: "#7a8494", padding: "8px 0" }}>Refreshing saved data…</div>}
            {!loading && artifacts.length === 0 && <div style={{ fontSize: 12, lineHeight: 1.5, color: "#7a8494", padding: "8px 0 12px" }}>No files are persisted for this case yet. Add files in the workspace and continue to save them.</div>}
            <div style={{ display: "grid", gap: 8 }}>
              {artifacts.map((artifact) => (
                <div key={artifact.id} style={{ border: "1px solid #edf0f4", borderRadius: 10, padding: "10px 9px", display: "flex", gap: 8, alignItems: "flex-start" }}>
                  <FileText size={16} style={{ color: "#5967c7", flexShrink: 0, marginTop: 1 }} />
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div title={artifact.filename} style={{ fontSize: 12, fontWeight: 650, overflowWrap: "anywhere" }}>{artifact.filename}</div>
                    <div style={{ marginTop: 3, fontSize: 10, color: "#7a8494" }}>{artifact.product} · {artifact.kind.replace(/_/g, " ")} · {formatBytes(artifact.size_bytes)}</div>
                    <div title={artifact.sha256} style={{ marginTop: 4, fontSize: 10, color: "#9aa2af", fontFamily: "ui-monospace, monospace" }}>SHA-256 {artifact.sha256.slice(0, 12)}…</div>
                  </div>
                  <button type="button" title="Delete local artifact" aria-label={`Delete ${artifact.filename}`} onClick={() => void removeArtifact(artifact)} style={{ border: 0, background: "transparent", color: "#98a1ae", cursor: "pointer", padding: 3, flexShrink: 0 }}><Trash2 size={14} /></button>
                </div>
              ))}
            </div>
            {message && <div role="status" style={{ marginTop: 12, fontSize: 11, lineHeight: 1.45, color: online ? "#667085" : "#a16207", overflowWrap: "anywhere" }}>{message}</div>}
          </div>
          <footer style={{ padding: "10px 16px", background: "#f8f9fb", borderTop: "1px solid #edf0f4", color: "#8a93a2", fontSize: 10 }}>Files stay on this machine unless you configure an external service.</footer>
        </section>
      )}
      <button type="button" style={buttonStyle} onClick={() => setExpanded((value) => !value)} aria-expanded={expanded}>
        <Database size={16} /> Local cases <span style={{ borderRadius: 999, background: online ? "#e1f5ed" : "#fef3c7", color: online ? "#167356" : "#92400e", padding: "3px 6px", fontSize: 10 }}>{cases.length}</span> {expanded ? <ChevronDown size={15} /> : <ChevronUp size={15} />}
      </button>
    </div>
  );
}
