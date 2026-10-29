"use client";

import { useEffect, useState, type FormEvent, type KeyboardEvent } from "react";
import { ArrowUpRight, Bot, FileSearch, MessageSquareText, Plus, Sparkles, UserRound } from "lucide-react";
import { getCase, listAgentMessages, sendAgentChat, type AgentMessage, type DiagnosticPreparationResponse } from "@/lib/atlas-api";

type AgentConversationProps = {
  onPrepare: (problemDescription: string) => Promise<DiagnosticPreparationResponse>;
  hasStagedFiles: boolean;
  onNewInvestigation: () => void;
};

type LocalRun = { id: string; description: string; result: DiagnosticPreparationResponse };

const ACTIVE_CASE_KEY = "atlas.agent.activeCaseId";
const SESSION_CASE_KEY = "atlas.agentSessionCaseId";
const QUICK_PROMPTS = [
  "Summarize the heap dump findings and likely causes",
  "Explain the Leak Suspects report in plain language",
  "Which classes or objects retain the most memory?",
  "Show the most important dominators and what they mean",
  "What paths to GC roots should I investigate next?",
  "Compare the current findings with a baseline heap dump",
  "Analyze the thread states and check for possible deadlocks",
  "What additional evidence or version details do you need from me?",
];

function newLocalId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") return crypto.randomUUID();
  return `run-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function AgentConversation({ onPrepare, hasStagedFiles, onNewInvestigation }: AgentConversationProps) {
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
    const restore = async () => {
      try {
        const storedId = window.localStorage.getItem(ACTIVE_CASE_KEY) || window.sessionStorage.getItem(SESSION_CASE_KEY);
        if (!storedId) return;
        const [record, history] = await Promise.all([getCase(storedId), listAgentMessages(storedId)]);
        if (cancelled) return;
        window.sessionStorage.setItem(SESSION_CASE_KEY, storedId);
        setCaseId(storedId);
        setIncidentRef(record.display_id);
        setMessages(history);
      } catch (cause) {
        if (!cancelled) setError(cause instanceof Error ? cause.message : "Could not restore the previous agent conversation.");
      } finally {
        if (!cancelled) setRestoring(false);
      }
    };
    void Promise.resolve().then(restore);
    return () => { cancelled = true; };
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
      // Initial intake or new uploads: persist evidence and run only the matching local plugin.
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
        <span className="agent-section-label">SUGGESTED QUESTIONS · 8 OPTIONS</span>
        <div className="agent-prompt-chips">
          {QUICK_PROMPTS.map((prompt) => (
            <button className="agent-prompt-chip" key={prompt} type="button" disabled={working || restoring} onClick={() => { setDraft(prompt); setStatus(""); setError(""); }}>
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
            </article>
          ))}
        </div>
      )}

      <form className="agent-composer" onSubmit={submitMessage}>
        <label className="agent-composer-label" htmlFor="atlas-agent-prompt">Problem description or follow-up question</label>
        <textarea
          id="atlas-agent-prompt"
          name="message"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleComposerKeyDown}
          rows={4}
          maxLength={12000}
          disabled={working || restoring}
          placeholder="Describe the symptoms, ask what the MAT report means, or answer the agent's follow-up question…"
        />
        <div className="agent-composer-footer">
          <span>Ctrl/⌘ + Enter to send · {draft.length}/12,000 {restoring ? "· restoring conversation…" : hasStagedFiles ? "· new files will be analyzed" : "· conversation history is saved locally"}</span>
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
