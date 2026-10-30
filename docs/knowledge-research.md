# Online technical knowledge research

## Status and scope

The online technical-research endpoint `POST /api/v1/knowledge/search` is available directly,
and approved technical chat turns also invoke the same configured `web` and `jira` providers
automatically after local classification and sanitization. The chat API returns bounded
research metadata; a dedicated reference panel in the frontend is a later UI increment.

Provider implementations are isolated behind the `KnowledgeProvider` Rust trait and the
provider registry in `crates/atlas-api/src/knowledge.rs`. This is a compile-time extension
contract; runtime-loaded plugins (for example WASM or JSON-RPC providers) are not implemented.
Red Hat KCS, Red Hat product documentation, upstream documentation indexes, GitHub issues,
and authenticated private Jira connectors are future provider increments, not current
capabilities.

## Technical-only classification

The technical-support policy uses the configured **local Ollama model**, not a product-name
or technology-keyword allowlist. The classifier asks whether the current user-authored request
is technical. For chat follow-ups it may also consider up to six previous **user-authored**
messages, each bounded to 1,200 characters. Assistant messages, assistant-generated summaries,
local diagnostic evidence, and attachment contents are not part of classifier history.

The classifier uses `ATLAS_LLM_BASE_URL` and `ATLAS_LLM_MODEL`, the same local Ollama settings
used by the agent. The URL must be plain HTTP on a loopback host; remote and LAN endpoints,
credentials, custom paths, query strings, and fragments are rejected. HTTP redirects and
environment proxies are disabled. The classifier has a bounded timeout and response size.
It accepts only a JSON object containing a boolean `technical` field. An unavailable model,
invalid response, or ambiguous decision fails closed before local evidence is collected or
an online provider is called. Non-technical requests return HTTP 422; classifier failures
return HTTP 503.

The classifier is local processing, not an online search provider. The current user-authored
message is sent to the configured loopback Ollama endpoint for classification. It is not sent
to a public search service at that stage.

## Online-search request and response

The endpoint accepts only these JSON fields:

| Field | Meaning |
| --- | --- |
| `query` | Required user-authored text, 1–1,500 characters. |
| `sources` | Optional provider IDs: `web` and/or `jira`. When omitted or empty, both are selected. |
| `max_results` | Optional result limit, clamped to 1–10 per provider. |

Unknown JSON fields are rejected. The request does not accept case IDs, artifact IDs, file
paths, attachment objects, local evidence, or report content. The endpoint classifies the
query locally, sanitizes the query, then sends only the sanitized current query to the selected
online providers.

The response includes the sanitized query, bounded result references, per-provider status,
and privacy flags. `attachment_content_accessed`, `attachment_content_sent`, and
`local_diagnostic_context_sent` are false for this endpoint. Provider response bodies are
bounded to 2 MiB; the outbound provider client has a 15-second timeout and disables redirects
and environment proxies. Result URLs are restricted to HTTP(S) and checked against local and
private address patterns before they are returned.

## Privacy boundary and sanitization

The online-research module receives plain user-authored query text. It has no `AppState`,
SQLite pool, filesystem handle, upload handle, artifact identifier, or diagnostic-workspace
handle. It does not read or transmit uploaded heap dumps, thread dumps, logs, archives,
sosreports, OpenShift must-gather bundles, MAT indexes, generated reports, or local diagnostic
context. It does not include assistant messages or assistant-generated summaries in online
search queries.

Before a query leaves the host, the sanitizer removes or replaces common sensitive forms:

- Credential labels and values, including password, token, API-key, secret, authorization,
  cookie, and common cloud credential fields. CLI flags, inline assignments, delimiters with
  surrounding spaces, bearer/basic credentials, and quoted values are handled. Known token
  formats and high-entropy credential-like tokens are also redacted.
- URL, email, IPv4/IPv6 literal, and common private-host forms.
- Common Unix, Windows, and UNC filesystem paths, file-like artifact names, long values,
  and long hexadecimal/UUID-like identifiers.
- Organization-specific terms and credential-field names supplied by the operator.

Configure additional redaction terms, credential keys, and private DNS suffixes with:

- `ATLAS_RESEARCH_REDACT_TERMS` — comma-separated customer names, tenant names, project names,
  host labels, or other organization-specific text that must not leave the machine.
- `ATLAS_RESEARCH_SECRET_KEYS` — comma-separated additional credential field names, such as
  organization-specific API token or client-secret labels.
- `ATLAS_RESEARCH_PRIVATE_HOST_SUFFIXES` — comma-separated private DNS suffixes, with or
  without a leading dot.

This sanitizer is defense in depth, not a guarantee that arbitrary confidential wording,
proprietary identifiers, or every secret format can be recognized. Operators should configure
organization-specific terms, review the returned `sanitized_query`, and avoid entering data
that must remain confidential. The sanitizer cannot infer all customer names or private values
without operator-provided policy.

## Local agent evidence versus public search

The regular diagnostic-agent chat path may attach bounded, locally collected evidence to its
Ollama answer request when `ATLAS_LLM_SEND_HEAP_CONTEXT=true` (the default when unset). That
is a separate local-only model interaction. The technical-scope classifier runs before local
evidence collection and receives no attachment or diagnostic context. The standalone endpoint sends only the sanitized current user query. Automatic chat research may
also use up to three prior user-authored turns after sanitization. Neither path sends the local
Ollama prompt, assistant response, persisted assistant history, report excerpts, or local evidence.

Raw heap dumps and MAT index/cache files are not included in the local agent context either.
See [Local agent setup](local-agent.md) for that separate boundary.

## Web-search providers

The `web` provider selects one configured backend:

- `ATLAS_BRAVE_SEARCH_API_KEY` — use Brave Search. If this variable is present, Brave is
  selected.
- `ATLAS_SEARXNG_URL` — use SearXNG when the Brave key is not configured. JSON output must
  be enabled. Remote SearXNG must use HTTPS; plain HTTP is accepted only for a loopback
  instance.

Keep the Brave credential in the backend environment. Never place it in a `NEXT_PUBLIC_*`
variable or commit it to source control. If neither provider is configured, web search is
reported as unavailable. A configured but failing Brave request is reported as unavailable;
it does not silently fall back to SearXNG.

## Jira provider

There are no implicit Jira host defaults. Configure only operator-approved instances with
`ATLAS_JIRA_INSTANCES`, a JSON array containing one to ten objects. Each object has:

- `name` — display name, 1–100 characters.
- `base_url` — HTTPS instance base URL, no embedded credentials, query, or fragment.
- `api_version` — `2` or `3`.

Example for two public issue trackers (enable only the instances approved for your use):

```bash
export ATLAS_JIRA_INSTANCES='[{"name":"Apache Jira","base_url":"https://issues.apache.org/jira","api_version":"2"},{"name":"Red Hat Jira","base_url":"https://issues.redhat.com","api_version":"2"}]'
```

The connector uses anonymous, read-only search and can return only issues visible without
authentication. It does not implement authenticated private/customer Jira access. API v2
uses `GET /rest/api/2/search`; API v3 uses `POST /rest/api/3/search/jql`. Jira query text is
built from sanitized terms, and issue descriptions are reduced to bounded text excerpts.

The URL checks reject non-HTTPS URLs, embedded credentials, local/private IP literals, and
recognized private hostname suffixes. They do not provide DNS pinning or replace outbound
network controls. Configure trusted Jira hosts and restrict host-level egress appropriately.

## Example request

The local Ollama classifier must be running with the configured model before this endpoint
can search. Configure at least one web provider and/or `ATLAS_JIRA_INSTANCES` first.

```bash
curl -fsS http://127.0.0.1:8080/api/v1/knowledge/search \
  -H 'Content-Type: application/json' \
  -d '{"query":"Kafka memory leak in version 3.7.0","sources":["web","jira"],"max_results":5}'
```

A non-technical query returns HTTP 422. An unavailable or invalid local classifier response
returns HTTP 503 and does not trigger external search. A JSON body with an unexpected field
(for example, `files`) is rejected by the strict request schema.

## Automatic chat research (Phase 2A)

After the local Ollama technical-topic classifier approves a chat turn, Atlas builds a bounded
research query from the current user-authored message and up to three recent prior user-authored
messages. Each message is sanitized locally before query composition. The classifier may use up
to six prior user messages; the online search query has a smaller three-message context window.
Assistant messages, assistant-generated summaries, uploaded files, report excerpts, diagnostic
context, artifact metadata, and filesystem paths are not query inputs.

The chat handler calls the same compiled `web` and `jira` provider adapters used by the standalone
research endpoint. Provider failures and missing provider configuration are returned as research
status metadata and do not stop local Ollama answer generation. A local classification failure
still fails closed with HTTP 503; a non-technical request is rejected with HTTP 422 before any
online search or local evidence collection.

At most six references are supplied to the local answer model, with bounded titles, URLs, and
snippets. A research status is always included, even when providers are unavailable or return no
references, so the model is not led to imply that research succeeded. Returned references are
labelled untrusted third-party material; the model is instructed not to follow embedded
instructions and to cite exact titles and URLs when relying on sources.
The chat API response includes a `research` object with `status`, `sanitized_query`, `results`,
per-provider status, and explicit privacy flags confirming that research did not access or send
attachment content, assistant message content, or local diagnostic context to online providers. The frontend renders references and provider status beneath the assistant message that used them; the backend stores each bounded research snapshot with that assistant message so references remain available after conversation restoration.

Local diagnostic evidence is collected only after the technical-topic gate and remains separate
from online query construction. Only sanitized user-authored text is sent to web/Jira. No uploads,
logs, heap dumps, thread dumps, sosreports, must-gather bundles, MAT indexes, assistant text,
reports, or local evidence are sent to those providers.

## Chat research references in conversation history (Phase 2B)

The agent chat response includes per-turn research metadata. Phase 2B stores a bounded JSON
snapshot of that metadata in the `research_json` column on the corresponding assistant message.
The nullable column was added with an additive SQLite migration; older messages remain valid and
return without a `research` property. Invalid legacy or manually edited JSON is omitted safely
rather than preventing conversation restoration.

The frontend renders a reference panel beneath the assistant answer that used the sources. It
shows provider availability, result counts, source type, provider name, title, bounded snippet,
and an optional provider-supplied date label. HTTP(S) links open in a new tab with `noopener`
and `noreferrer`; unsupported schemes and URLs with embedded credentials are rendered as
non-clickable text. The sanitized outbound query is available in a collapsed disclosure.

Reference metadata is a local conversation record. Persisting it does not change the online
privacy boundary: provider inputs remain sanitized user-authored text only. No attachments,
assistant text, local diagnostic evidence, logs, dumps, reports, or MAT indexes are stored in
research metadata. External snippets and titles remain untrusted third-party content.

## Operations and current limitations

The API currently has no authentication or multi-user authorization. Keep it bound to loopback
and do not expose it directly to a network. Search results and snippets are untrusted external
content: they are references for the local agent/user to review, not instructions and not
permission to execute commands or tools.

Current online providers are compile-time `web` and `jira` adapters only. There is no research
result cache and no Red Hat KCS, product-documentation, upstream-docs, GitHub issue, or authenticated
private Jira provider yet. Chat-to-search orchestration is implemented for the current providers.
