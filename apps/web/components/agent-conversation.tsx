"use client";

import { useEffect, useState, type FormEvent, type KeyboardEvent } from "react";
import { ArrowUpRight, Bot, ExternalLink, FileSearch, FileText, MessageSquareText, Plus, Sparkles, UserRound, X } from "lucide-react";
import { createCase, getCase, listAgentMessages, sendAgentChat, type AgentMessage, type AgentResearch, type DiagnosticPreparationResponse } from "@/lib/atlas-api";

type StagedAttachment = { id: string; name: string; size: number };

type AgentConversationProps = {
  onPrepare: (problemDescription: string) => Promise<DiagnosticPreparationResponse>;
  hasStagedFiles: boolean;
  stagedFiles: StagedAttachment[];
  onFilesSelected: (files: FileList) => void;
  onRemoveStagedFile: (id: string) => void;
  onNewInvestigation: () => void;
};

type LocalRun = { id: string; description: string; result: DiagnosticPreparationResponse };

const ACTIVE_CASE_KEY = "atlas.agent.activeCaseId";
const SESSION_CASE_KEY = "atlas.agentSessionCaseId";
const RESTORE_TIMEOUT_MS = 8000;
const QUICK_PROMPTS = [
  "Analyze this heap dump for memory-leak suspects",
  "Review this thread dump for blocked threads or deadlocks",
  "Explain the findings and recommend the next diagnostic steps",
];

function newLocalId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") return crypto.randomUUID();
  return `run-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/** Accept only absolute HTTP(S) URLs without embedded credentials for external navigation. */
function safeResearchHref(value: string): string | null {
  try {
    const url = new URL(value);
    if ((url.protocol !== "https:" && url.protocol !== "http:") || url.username || url.password) return null;
    return url.toString();
  } catch {
    return null;
  }
}

/** Converts provider status codes into concise labels for screen readers and users. */
function researchStatusLabel(status: string): string {
  switch (status) {
    case "results": return "References found";
    case "no_results": return "No references returned";
    case "unavailable": return "Research unavailable; local answer is still available";
    case "skipped_sensitive_query": return "Research skipped because too little safe query text remained";
    default: return "Research status unavailable";
  }
}

/** Renders provider health and safe, clickable references for one assistant answer. */
function ResearchReferences({ research }: { research?: AgentResearch | null }) {
  if (!research) return null;

  const results = Array.isArray(research.results) ? research.results : [];
  const providers = Array.isArray(research.providers) ? research.providers : [];
  const statusTone = research.status === "results" ? "success"
    : research.status === "unavailable" ? "warning" : "neutral";

  return (
    <section className="agent-research-panel" aria-label="Online research references">
      <div className="agent-research-heading">
        <div>
          <strong>Online research</strong>
          <span className={`agent-research-state agent-research-state-${statusTone}`}>{researchStatusLabel(research.status)}</span>
        </div>
        <span className="agent-research-count">{results.length} source{results.length === 1 ? "" : "s"}</span>
      </div>

      {providers.length > 0 && (
        <div className="agent-research-providers" aria-label="Provider status">
          {providers.map((provider, index) => (
            <span className={`agent-research-provider ${provider.status === "ok" ? "is-available" : "is-unavailable"}`} key={`${provider.provider}-${index}`}>
              <strong>{provider.provider}</strong>
              <span>{provider.status === "ok" ? "Available" : provider.status === "misconfigured" ? "Configuration needed" : "Unavailable"}</span>
              <span className="agent-research-provider-count">{provider.result_count} result{provider.result_count === 1 ? "" : "s"}</span>
              {provider.message && <span className="agent-research-provider-message">{provider.message}</span>}
            </span>
          ))}
        </div>
      )}

      {results.length > 0 ? (
        <ol className="agent-research-results">
          {results.map((reference, index) => {
            const href = safeResearchHref(reference.url);
            const sourceKind = reference.source_type === "jira" ? "Jira issue"
              : reference.source_type === "web" ? "Web source" : reference.source_type;
            return (
              <li className="agent-research-result" key={`${reference.provider}-${reference.url}-${index}`}>
                <div className="agent-research-result-meta">
                  <span>{sourceKind}</span>
                  <span>{reference.provider}</span>
                  {reference.updated_at && <span>Source date: {reference.updated_at.slice(0, 80)}</span>}
                </div>
                {href ? (
                  <a href={href} target="_blank" rel="noopener noreferrer nofollow" className="agent-research-link">
                    <span>{reference.title || reference.url}</span><ExternalLink size={12} aria-hidden="true" />
                  </a>
                ) : (
                  <span className="agent-research-link-disabled">{reference.title || "Reference URL unavailable"}</span>
                )}
                {reference.snippet && <p className="agent-research-snippet">{reference.snippet}</p>}
              </li>
            );
          })}
        </ol>
      ) : (
        <p className="agent-research-empty">{researchStatusLabel(research.status)}. The agent can still answer using local reasoning and any enabled local diagnostic evidence.</p>
      )}

      {research.sanitized_query && (
        <details className="agent-research-query">
          <summary>View sanitized search query</summary>
          <p>{research.sanitized_query}</p>
        </details>
      )}
    </section>
  );
}

export function AgentConversation({ onPrepare, hasStagedFiles, stagedFiles, onFilesSelected, onRemoveStagedFile, onNewInvestigation }: AgentConversationProps) {
  const [draft, setDraft] = useState("");
  const [caseId, setCaseId] = useState<string | null>(null);
  const [incidentRef, setIncidentRef] = useState("");
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [runs, setRuns] = useState<LocalRun[]>([]);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const [restoring, setRestoring] = useState(true);

  useEffect(() => {
    let cancelled = false;
    let selectionGeneration = 0;
    const loadCase = async (storedId: string) => {
      const requestGeneration = ++selectionGeneration;
      let restoreTimer: number | undefined;
      try {
        const [record, history] = await Promise.race([
          Promise.all([getCase(storedId), listAgentMessages(storedId)]),
          new Promise<never>((_, reject) => {
            restoreTimer = window.setTimeout(() => {
              reject(new Error(`Loading the saved conversation timed out after ${RESTORE_TIMEOUT_MS / 1000} seconds.`));
            }, RESTORE_TIMEOUT_MS);
          }),
        ]);
        if (cancelled || requestGeneration !== selectionGeneration) return;
        try {
          window.localStorage.setItem(ACTIVE_CASE_KEY, storedId);
          window.sessionStorage.setItem(SESSION_CASE_KEY, storedId);
        } catch { /* The API remains authoritative if browser storage is disabled. */ }
        setCaseId(storedId);
        setIncidentRef(record.display_id);
        setMessages(history);
        setError("");
      } catch (cause) {
        if (!cancelled && requestGeneration === selectionGeneration) {
          const detail = cause instanceof Error ? cause.message : "The saved conversation could not be loaded.";
          setCaseId(null);
          setIncidentRef("");
          setMessages([]);
          setRuns([]);
          setError(`${detail} You can continue with a new investigation or select a saved incident from Recent Incidents.`);
          // Clear only the stale browser selection pointer; the incident and its files remain in SQLite.
          try {
            if (window.localStorage.getItem(ACTIVE_CASE_KEY) === storedId) {
              window.localStorage.removeItem(ACTIVE_CASE_KEY);
            }
            if (window.sessionStorage.getItem(SESSION_CASE_KEY) === storedId) {
              window.sessionStorage.removeItem(SESSION_CASE_KEY);
            }
          } catch { /* Storage can be disabled by browser policy. */ }
        }
      } finally {
        if (restoreTimer !== undefined) window.clearTimeout(restoreTimer);
        if (!cancelled && requestGeneration === selectionGeneration) setRestoring(false);
      }
    };
    const restore = async () => {
      let storedId: string | null = null;
      try {
        storedId = window.localStorage.getItem(ACTIVE_CASE_KEY) || window.sessionStorage.getItem(SESSION_CASE_KEY);
      } catch { /* Storage can be disabled by browser policy. */ }
      if (storedId) await loadCase(storedId);
      else if (!cancelled) setRestoring(false);
    };
    const onCaseSelected = (event: Event) => {
      const selectedId = (event as CustomEvent<{ caseId?: string }>).detail?.caseId;
      if (!selectedId) return;
      setRestoring(true);
      setStatus("");
      setError("");
      setRuns([]);
      setMessages([]);
      setCaseId(selectedId);
      setIncidentRef("");
      void loadCase(selectedId);
    };
    const onNewChat = () => {
      selectionGeneration += 1;
      setCaseId(null);
      setIncidentRef("");
      setMessages([]);
      setRuns([]);
      setDraft("");
      setStatus("");
      setError("");
      setRestoring(false);
    };
    window.addEventListener("atlas:case-selected", onCaseSelected);
    window.addEventListener("atlas:new-chat", onNewChat);
    void Promise.resolve().then(restore);
    return () => {
      cancelled = true;
      selectionGeneration += 1;
      window.removeEventListener("atlas:case-selected", onCaseSelected);
      window.removeEventListener("atlas:new-chat", onNewChat);
    };
  }, []);

  async function submitMessage(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const content = draft.trim();
    if (!content || working || restoring) return;
    setWorking(true);
    setStatus("");
    setError("");
    let currentCaseId = caseId || window.localStorage.getItem(ACTIVE_CASE_KEY) || window.sessionStorage.getItem(SESSION_CASE_KEY);
    try {
      // A conversation can start with text alone; files are optional until a diagnostic run is requested.
      if (!currentCaseId && !hasStagedFiles) {
        const title = `Incident: ${content.replace(/\s+/g, " ")}`.slice(0, 140);
        const created = await createCase(title);
        currentCaseId = created.id;
        setCaseId(currentCaseId);
        setIncidentRef(created.display_id);
        window.localStorage.setItem(ACTIVE_CASE_KEY, currentCaseId);
        window.sessionStorage.setItem(SESSION_CASE_KEY, currentCaseId);
        window.dispatchEvent(new CustomEvent("atlas:case-saved", { detail: { caseId: currentCaseId } }));
      }
      // When new files are attached, save them and run only the matching local diagnostic plugin.
      if (!currentCaseId || hasStagedFiles) {
        const result = await onPrepare(content);
        currentCaseId = result.case_id;
        setCaseId(currentCaseId);
        window.localStorage.setItem(ACTIVE_CASE_KEY, currentCaseId);
        window.sessionStorage.setItem(SESSION_CASE_KEY, currentCaseId);
        const record = await getCase(currentCaseId);
        setIncidentRef(record.display_id);
        setRuns((existing) => [{ id: newLocalId(), description: content, result }, ...existing].slice(0, 8));
      }
      if (!currentCaseId) throw new Error("Atlas did not create an internal incident workspace.");
      setStatus("Sending the problem and bounded local evidence to local Ollama…");
      const response = await sendAgentChat(currentCaseId, content);
      setIncidentRef(response.incident_ref);
      setMessages(response.messages);
      setDraft("");
      setStatus(response.needs_more_info
        ? "The agent needs more information. Reply below to continue this investigation."
        : response.evidence_attached
          ? "Analysis updated using the latest local evidence. Ask a follow-up to continue the same conversation."
          : "Response received. Follow-up messages continue the same conversation.");
    } catch (cause) {
      setDraft(content);
      setError(cause instanceof Error ? cause.message : "The diagnostic agent could not complete this request.");
    } finally {
      setWorking(false);
    }
  }

  function handleComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  }

  function startNewInvestigation() {
    window.localStorage.removeItem(ACTIVE_CASE_KEY);
    window.sessionStorage.removeItem(SESSION_CASE_KEY);
    setCaseId(null);
    setIncidentRef("");
    setMessages([]);
    setRuns([]);
    setDraft("");
    setStatus("New investigation ready. Describe the problem and attach diagnostic evidence.");
    setError("");
    onNewInvestigation();
  }

  return (
    <section className="agent-conversation" aria-labelledby="agent-conversation-title" data-agent-conversation>
      <header className="agent-conversation-header">
        <div className="agent-conversation-icon"><MessageSquareText size={19} /></div>
        <div className="agent-conversation-heading">
          <div className="agent-conversation-title-row">
            <h2 id="agent-conversation-title">Middleware Atlas agent</h2>
            <span className="agent-preview-badge"><Sparkles size={12} /> Evidence-led chat</span>
          </div>
          <p>Describe the incident, attach dumps, and continue asking questions in one remembered conversation.</p>
          {incidentRef && <p className="agent-incident-ref">Investigation reference: <strong>{incidentRef}</strong></p>}
        </div>
        <button className="agent-new-investigation" type="button" onClick={startNewInvestigation} disabled={working} title="Start a new internal investigation">
          <Plus size={14} /> New investigation
        </button>
      </header>

      <div className="agent-connection-notice" role="note">
        <div className="agent-connection-icon"><Bot size={18} /></div>
        <div>
          <strong>Local diagnostics with persistent agent conversation</strong>
          <p>Heap and thread dumps are analyzed locally first. Atlas uses a loopback-only Ollama model and sends only bounded text/CSV evidence to that local model; raw heap dumps and MAT indexes remain on this machine.</p>
        </div>
      </div>

      <div className="agent-quick-prompts">
        <span className="agent-section-label">SUGGESTED STARTING POINTS</span>
        <div className="agent-prompt-chips">
          {QUICK_PROMPTS.map((prompt) => (
            <button className="agent-prompt-chip" key={prompt} type="button" disabled={working} onClick={() => { setDraft(prompt); setStatus(""); setError(""); }}>
              {prompt}
            </button>
          ))}
        </div>
      </div>

      {(runs.length > 0 || messages.length > 0) && (
        <div className="agent-chat-history" aria-label="Investigation conversation history">
          <div className="agent-section-label">CONVERSATION HISTORY</div>
          {runs.map((run) => (
            <article className="agent-run-summary" key={run.id}>
              <div className="agent-run-summary-heading"><FileSearch size={14} /><strong>Local diagnostic run</strong><span>{run.result.status}</span></div>
              <p>{run.description}</p>
              {run.result.preparations.map((prepared) => (
                <div className="agent-run-plugin" key={prepared.tool_run_id}>
                  <div><strong>{prepared.tool_id}</strong> · {prepared.status}</div>
                  <p>{prepared.artifact_filename}: {prepared.summary}</p>
                  {prepared.warnings.map((warning) => <p className="agent-preparation-warning" key={warning}>{warning}</p>)}
                  <p className="agent-local-output-note">Files saved in the local investigation workspace.</p>
                </div>
              ))}
            </article>
          ))}
          {messages.map((message) => (
            <article className={`agent-chat-message ${message.role === "assistant" ? "agent-chat-assistant" : "agent-chat-user"}`} key={message.id}>
              <div className="agent-chat-message-heading">
                {message.role === "assistant" ? <Bot size={14} /> : <UserRound size={14} />}
                <strong>{message.role === "assistant" ? "Atlas" : "You"}</strong>
                <time dateTime={message.created_at}>{new Date(message.created_at).toLocaleString()}</time>
              </div>
              <p>{message.content}</p>
              {message.role === "assistant" && <ResearchReferences research={message.research} />}
            </article>
          ))}
        </div>
      )}

      {caseId && runs.length === 0 && messages.length === 0 && !restoring && (
        <div className="agent-empty-thread" role="status">
          <div className="agent-empty-thread-icon"><MessageSquareText size={17} /></div>
          <div>
            <strong>{incidentRef ? `You're viewing ${incidentRef}` : "Incident conversation"}</strong>
            <p>This incident has no saved chat turns yet. Continue here to start or resume the conversation; its saved artifacts and diagnostic outputs remain attached to this case.</p>
          </div>
        </div>
      )}

      <form className="agent-composer" onSubmit={submitMessage}>
        {stagedFiles.length > 0 && (
          <div className="agent-attachments" aria-label="Files ready to attach">
            {stagedFiles.map((file) => (
              <span className="agent-attachment-chip" key={file.id} title={`${file.name} · ${file.size.toLocaleString()} bytes`}>
                <FileText size={13} />
                <span>{file.name}</span>
                <button type="button" aria-label={`Remove ${file.name}`} title={`Remove ${file.name}`} onClick={() => onRemoveStagedFile(file.id)} disabled={working}>
                  <X size={12} />
                </button>
              </span>
            ))}
          </div>
        )}
        <label className="agent-composer-label" htmlFor="atlas-agent-prompt">Problem description or follow-up question</label>
        <textarea
          id="atlas-agent-prompt"
          name="message"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleComposerKeyDown}
          rows={4}
          maxLength={12000}
          disabled={working}
          placeholder="Describe the symptoms, ask what the MAT report means, or answer the agent's follow-up question…"
        />
        <div className="agent-composer-footer">
          <div className="agent-composer-tools">
            <label className={`agent-attach-button ${working ? "is-disabled" : ""}`} title="Attach a heap or thread dump">
              <Plus size={19} aria-hidden="true" />
              <span className="visually-hidden">Attach diagnostic files</span>
              <input
                className="agent-file-picker-input"
                type="file"
                multiple
                disabled={working}
                aria-label="Attach diagnostic files"
                onChange={(event) => {
                  const selected = event.currentTarget.files;
                  if (selected && selected.length > 0) onFilesSelected(selected);
                  event.currentTarget.value = "";
                }}
              />
            </label>
            <span className="agent-composer-hint">Ctrl/⌘ + Enter · {draft.length}/12,000 {restoring ? "· loading incident…" : hasStagedFiles ? `· ${stagedFiles.length} file(s) ready` : "· saved locally"}</span>
          </div>
          <button className="button button-primary agent-queue-button" type="submit" disabled={!draft.trim() || working || restoring}>
            {working ? "Working…" : "Send to Atlas"} {!working && <ArrowUpRight size={15} />}
          </button>
        </div>
      </form>
      {status && <p className="agent-composer-status" role="status" aria-live="polite">{status}</p>}
      {error && <p className="agent-composer-error" role="alert">{error}</p>}
      <p className="agent-session-note">Conversation messages are stored with the internal incident on this machine. Atlas stores conversation turns locally; you never need to enter an internal case ID.</p>
    </section>
  );
}
