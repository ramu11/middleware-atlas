"use client";

import { useState, type FormEvent, type KeyboardEvent } from "react";
import { ArrowUpRight, Bot, MessageSquareText, Sparkles } from "lucide-react";

type PromptDraft = {
  id: string;
  content: string;
};

const QUICK_PROMPTS = [
  "Summarize the evidence collected so far",
  "Help me investigate Kafka replication lag",
  "What should I inspect in a Java thread dump?",
];

function newDraftId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `prompt-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function AgentConversation() {
  const [draft, setDraft] = useState("");
  const [queuedPrompts, setQueuedPrompts] = useState<PromptDraft[]>([]);
  const [status, setStatus] = useState("");

  function queuePrompt(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const content = draft.trim();
    if (!content) return;

    setQueuedPrompts((current) => [...current, { id: newDraftId(), content }]);
    setDraft("");
    setStatus(
      "Prompt added to this page session only. The agent conversation API is not connected yet, so nothing was sent to a model.",
    );
  }

  function handleComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  }

  return (
    <section
      className="agent-conversation"
      aria-labelledby="agent-conversation-title"
      data-agent-conversation-preview
    >
      <header className="agent-conversation-header">
        <div className="agent-conversation-icon"><MessageSquareText size={19} /></div>
        <div className="agent-conversation-heading">
          <div className="agent-conversation-title-row">
            <h2 id="agent-conversation-title">Agent conversation</h2>
            <span className="agent-preview-badge"><Sparkles size={12} /> UI preview</span>
          </div>
          <p>Ask questions about your incident and the diagnostic evidence.</p>
        </div>
      </header>

      <div className="agent-connection-notice" role="note">
        <div className="agent-connection-icon"><Bot size={18} /></div>
        <div>
          <strong>Agent endpoint not connected yet</strong>
          <p>
            The conversation composer is ready for integration. For now, prompts stay in this page session and are not sent to Dify or a model.
          </p>
        </div>
      </div>

      <div className="agent-quick-prompts">
        <span className="agent-section-label">START WITH A QUESTION</span>
        <div className="agent-prompt-chips">
          {QUICK_PROMPTS.map((prompt) => (
            <button
              className="agent-prompt-chip"
              key={prompt}
              type="button"
              onClick={() => {
                setDraft(prompt);
                setStatus("");
              }}
            >
              {prompt}
            </button>
          ))}
        </div>
      </div>

      {queuedPrompts.length > 0 && (
        <div className="agent-queued-prompts" aria-label="Local prompt drafts">
          <div className="agent-section-label">LOCAL DRAFTS · NOT SENT</div>
          {queuedPrompts.map((prompt) => (
            <article className="agent-draft-message" key={prompt.id}>
              <div className="agent-draft-label">You · draft</div>
              <p>{prompt.content}</p>
            </article>
          ))}
        </div>
      )}

      <form className="agent-composer" onSubmit={queuePrompt}>
        <label className="agent-composer-label" htmlFor="atlas-agent-prompt">Your message</label>
        <textarea
          id="atlas-agent-prompt"
          name="message"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleComposerKeyDown}
          rows={4}
          maxLength={12000}
          placeholder="Ask about broker errors, replication lag, thread contention, heap usage…"
        />
        <div className="agent-composer-footer">
          <span>Ctrl/⌘ + Enter for a new local draft · {draft.length}/12,000</span>
          <button className="button button-primary agent-queue-button" type="submit" disabled={!draft.trim()}>
            Queue prompt <ArrowUpRight size={15} />
          </button>
        </div>
      </form>
      {status && <p className="agent-composer-status" role="status" aria-live="polite">{status}</p>}
    </section>
  );
}
