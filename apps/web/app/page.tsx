"use client";

import {
  Activity, ArrowDownToLine, Atom, Boxes,
  Check, ChevronDown, CircleHelp, Clock3,
  CloudUpload, Command, FileArchive, FileCode2,
  FileSearch, FileText, FolderOpen, HardDrive,
  Layers3, LifeBuoy, Menu, Network,
  Plus, Search, Server, ShieldCheck,
  Trash2, Upload, X,
} from "lucide-react";
import { useCallback, useMemo, useRef, useState, type ChangeEvent, type DragEvent } from "react";
import { sha256Blob } from "@/lib/sha256-stream";
import { createCase, listArtifacts, listCases, prepareDiagnostics, uploadArtifact, type AtlasArtifact } from "@/lib/atlas-api";
import { AgentConversation } from "@/components/agent-conversation";

type ArtifactKind =
  | "Kafka logs" | "Artemis logs" | "Heap dump" | "Thread dump"
  | "sosreport" | "OpenShift" | "Debezium" | "Archive" | "Log / text" | "Other";

type Artifact = {
  id: string;
  file: File;
  kind: ArtifactKind;
};

type UploadState = {
  status: "uploading" | "failed";
  progress: number;
  error?: string;
};

function createArtifactId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `artifact-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function displayKindFromApi(kind: string): ArtifactKind {
  switch (kind) {
    case "kafka_log": return "Kafka logs";
    case "artemis_log": return "Artemis logs";
    case "heap_dump": return "Heap dump";
    case "thread_dump": return "Thread dump";
    case "sosreport": return "sosreport";
    case "openshift_bundle": return "OpenShift";
    case "debezium": return "Debezium";
    case "archive": return "Archive";
    case "log": return "Log / text";
    default: return "Other";
  }
}

const kindStyles: Record<ArtifactKind, string> = {
  "Kafka logs": "kind-kafka",
  "Artemis logs": "kind-artemis",
  "Heap dump": "kind-heap",
  "Thread dump": "kind-thread",
  sosreport: "kind-sos",
  OpenShift: "kind-openshift",
  Debezium: "kind-debezium",
  Archive: "kind-archive",
  "Log / text": "kind-other",
  Other: "kind-other",
};

function classify(name: string): ArtifactKind {
  // Keep preview labels aligned with config/default.toml; the API's result is canonical after upload.
  const n = name.toLowerCase();
  if ([".hprof", ".heapdump", ".phd"].some((extension) => n.endsWith(extension))) return "Heap dump";
  if (["thread", "javacore", "threaddump"].some((token) => n.includes(token)) && [".txt", ".log", ".dump", ".tdump"].some((extension) => n.endsWith(extension))) return "Thread dump";
  if (n.includes("sosreport") || n.endsWith(".sos")) return "sosreport";
  if (n.includes("debezium")) return "Debezium";
  if (n.includes("artemis") || n.includes("activemq")) return "Artemis logs";
  if (n.includes("kafka") || n.includes("mirrormaker") || n.includes("mirror-maker")) return "Kafka logs";
  if (n.includes("openshift") || n.includes("must-gather") || n.includes("must_gather")) return "OpenShift";
  if ([".tar.zst", ".zip", ".tar", ".tgz", ".gz", ".7z", ".xz", ".bz2", ".zst"].some((extension) => n.endsWith(extension))) return "Archive";
  if ([".log", ".txt", ".out", ".err"].some((extension) => n.endsWith(extension))) return "Log / text";
  return "Other";
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 4);
  return `${(bytes / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${units[i]}`;
}

function FileGlyph({ kind }: { kind: ArtifactKind }) {
  if (kind === "Heap dump" || kind === "Thread dump") {
    return <FileSearch size={19} />;
  }
  if (kind === "Archive" || kind === "sosreport") {
    return <FileArchive size={19} />;
  }
  if (kind === "Kafka logs" || kind === "Artemis logs" || kind === "Debezium") {
    return <FileCode2 size={19} />;
  }
  return <FileText size={19} />;
}

export default function Home() {
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [persistedArtifacts, setPersistedArtifacts] = useState<AtlasArtifact[]>([]);
  const [uploadStates, setUploadStates] = useState<Record<string, UploadState>>({});
  const [dragging, setDragging] = useState(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("All files");
  const [notice, setNotice] = useState("");
  const [showMenu, setShowMenu] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const [agentSessionCaseId, setAgentSessionCaseId] = useState<string | null>(null);

  const addFiles = useCallback((incoming: FileList | File[]) => {
    const selected = Array.from(incoming);
    if (!selected.length) return;

    // Never discard distinct files based on filename/size/mtime; those values can collide.
    const additions = selected.map((file) => ({
      id: createArtifactId(),
      file,
      kind: classify(file.name),
    }));
    setArtifacts((current) => [...current, ...additions]);
    setNotice("");
  }, []);

  const onInputChange = (event: ChangeEvent<HTMLInputElement>) => {
    if (event.target.files) addFiles(event.target.files);
    event.target.value = "";
  };

  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setDragging(false);
    if (event.dataTransfer.files) addFiles(event.dataTransfer.files);
  };

  const removeFile = (id: string) => {
    setArtifacts((current) => current.filter((item) => item.id !== id));
  };

  const visibleFiles = useMemo(() => artifacts.filter((item) => {
    const matchesQuery = item.file.name.toLowerCase().includes(query.toLowerCase());
    const matchesFilter = filter === "All files" || item.kind === filter;
    return matchesQuery && matchesFilter;
  }), [artifacts, filter, query]);
  const visiblePersistedArtifacts = useMemo(() => persistedArtifacts.filter((item) => {
    const matchesQuery = item.filename.toLowerCase().includes(query.toLowerCase());
    const matchesFilter = filter === "All files" || displayKindFromApi(item.kind) === filter;
    return matchesQuery && matchesFilter;
  }), [persistedArtifacts, filter, query]);

  const totalBytes = artifacts.reduce((sum, item) => sum + item.file.size, 0)
    + persistedArtifacts.reduce((sum, item) => sum + item.size_bytes, 0);
  const totalArtifactCount = artifacts.length + persistedArtifacts.length;
  const categories = [...new Set([
    ...artifacts.map((item) => item.kind),
    ...persistedArtifacts.map((item) => displayKindFromApi(item.kind)),
  ])];

    const [persistenceBusy, setPersistenceBusy] = useState(false);
  const [persistenceMessage, setPersistenceMessage] = useState("");



  const handlePrepareDiagnostics = async (problemDescription: string) => {
    const problem = problemDescription.trim();
    if (!problem) throw new Error("Describe the problem before starting analysis.");
    if (persistenceBusy) throw new Error("An upload or save is already in progress. Wait for it to finish, then retry.");

    // The case ID is an internal implementation detail. Start a private incident
    // workspace automatically for this conversation; never ask the customer for an ID.
    const isDiagnosticDump = (kind: string) => kind === "Heap dump" || kind === "Thread dump";
    let caseId: string | null = agentSessionCaseId;
    try {
      const restoredCaseId = window.localStorage.getItem("atlas.agent.activeCaseId");
      if (restoredCaseId) { caseId = restoredCaseId; setAgentSessionCaseId(restoredCaseId); window.sessionStorage.setItem("atlas.agentSessionCaseId", restoredCaseId); }
    } catch { /* storage can be disabled by browser policy */ }
    if (!caseId) {
      try { caseId = window.sessionStorage.getItem("atlas.agentSessionCaseId"); } catch { /* session persistence may be disabled */ }
      if (caseId) setAgentSessionCaseId(caseId);
    }
    setPersistenceBusy(true);
    setPersistenceMessage("Preparing the incident workspace and saving uploaded evidence…");

    try {
      const computedDigests = new Map<string, string>();
      let savedArtifacts: AtlasArtifact[] = caseId ? await listArtifacts(caseId) : [];
      if (!caseId && artifacts.some((item) => isDiagnosticDump(item.kind))) {
        // Look through existing internal incidents before creating another case or re-uploading a dump.
        // Filename/kind/size narrow candidates first; only candidates are hashed, in a stream.
        const previousCases = await listCases();
        const previousArtifactSets = await Promise.all(previousCases.map(async (previousCase) => ({
          caseId: previousCase.id,
          artifacts: await listArtifacts(previousCase.id),
        })));
        for (const item of artifacts.filter((candidate) => isDiagnosticDump(candidate.kind))) {
          const expectedKind = item.kind === "Heap dump" ? "heap_dump" : "thread_dump";
          const sameSizeCandidates = previousArtifactSets.flatMap((set) => set.artifacts
            .filter((artifact) => artifact.kind === expectedKind
              && artifact.filename === item.file.name
              && artifact.size_bytes === item.file.size)
            .map((artifact) => ({ caseId: set.caseId, artifact, artifacts: set.artifacts })));
          if (sameSizeCandidates.length === 0) continue;
          setPersistenceMessage(`Checking whether ${item.file.name} already exists in local storage…`);
          let lastHashPercent = -1;
          const digest = await sha256Blob(item.file, (percent) => {
            if (percent !== lastHashPercent) {
              lastHashPercent = percent;
              setPersistenceMessage(`Checking ${item.file.name} for an existing copy · ${percent}%`);
            }
          });
          computedDigests.set(item.id, digest);
          const existing = sameSizeCandidates.find((candidate) =>
            candidate.artifact.sha256.toLowerCase() === digest.toLowerCase()
          );
          if (existing) {
            caseId = existing.caseId;
            savedArtifacts = existing.artifacts;
            setAgentSessionCaseId(existing.caseId);
            try { window.sessionStorage.setItem("atlas.agentSessionCaseId", existing.caseId); window.localStorage.setItem("atlas.agent.activeCaseId", existing.caseId); } catch { /* session persistence may be disabled */ }
            setPersistedArtifacts(savedArtifacts);
            setPersistenceMessage(`${item.file.name} already exists locally; reusing its internal incident and saved artifact.`);
            break;
          }
        }
      }
      if (!caseId) {
        if (!artifacts.some((item) => isDiagnosticDump(item.kind))) {
          throw new Error("Attach a recognized heap dump (.hprof, .phd, .heapdump) or thread dump before starting a new analysis.");
        }
        const compactProblem = problem.replace(/\s+/g, " ");
        const title = `Incident: ${compactProblem}`.slice(0, 140);
        const created = await createCase(title);
        caseId = created.id;
        setAgentSessionCaseId(created.id);
        try { window.localStorage.setItem("atlas.agent.activeCaseId", created.id); } catch { /* local session storage may be disabled */ }
        try { window.sessionStorage.setItem("atlas.agentSessionCaseId", created.id); } catch { /* session persistence may be disabled */ }
        setPersistedArtifacts([]);
      }

      const targetCaseId = caseId;
      if (!targetCaseId) throw new Error("Atlas could not create the internal incident workspace.");

      // Upload all staged evidence sequentially into the internally-created incident.
      // Successful files are removed from the staged queue, so a retry after partial
      // failure reuses the same internal case and does not upload successful files twice.
      const newlyUploadedArtifactIds: string[] = [];
      for (const item of artifacts) {
        const key = item.id;
        // Skip an artifact only after its full content SHA-256 matches an artifact already saved in this incident.
        // Hash only when an existing artifact has the same byte size; different sizes cannot be identical files.
        const expectedKind = item.kind === "Heap dump" ? "heap_dump" : item.kind === "Thread dump" ? "thread_dump" : null;
        const sameSizeCandidates = savedArtifacts.filter((artifact) =>
          artifact.filename === item.file.name
          && artifact.size_bytes === item.file.size
          && (expectedKind === null || artifact.kind === expectedKind)
        );
        if (sameSizeCandidates.length > 0) {
          setPersistenceMessage(`Checking whether ${item.file.name} is already stored…`);
          let digest = computedDigests.get(item.id);
          if (!digest) {
            let lastHashPercent = -1;
            digest = await sha256Blob(item.file, (percent) => {
              if (percent !== lastHashPercent) {
                lastHashPercent = percent;
                setPersistenceMessage(`Checking ${item.file.name} for a duplicate · ${percent}%`);
              }
            });
            computedDigests.set(item.id, digest);
          }
          const existing = sameSizeCandidates.find((artifact) => artifact.sha256.toLowerCase() === digest.toLowerCase());
          if (existing) {
            savedArtifacts = [existing, ...savedArtifacts.filter((artifact) => artifact.id !== existing.id)];
            newlyUploadedArtifactIds.push(existing.id);
            setPersistedArtifacts((current) => [existing, ...current.filter((artifact) => artifact.id !== existing.id)]);
            setArtifacts((current) => current.filter((candidate) => candidate.id !== item.id));
            setUploadStates((current) => {
              const next = { ...current };
              delete next[key];
              return next;
            });
            setPersistenceMessage(`${item.file.name} already exists in this incident; reused the saved artifact without uploading it again.`);
            continue;
          }
        }
        setUploadStates((current) => ({ ...current, [key]: { status: "uploading", progress: 0 } }));
        setPersistenceMessage(`Saving ${item.file.name} to the incident workspace…`);
        try {
          const saved = await uploadArtifact(targetCaseId, item.file, (percent) => {
            setUploadStates((current) => ({ ...current, [key]: { status: "uploading", progress: percent } }));
            setPersistenceMessage(`Uploading ${item.file.name} · ${percent}%`);
          });
          savedArtifacts = [saved, ...savedArtifacts.filter((artifact) => artifact.id !== saved.id)];
          newlyUploadedArtifactIds.push(saved.id);
          setPersistedArtifacts((current) => [saved, ...current.filter((artifact) => artifact.id !== saved.id)]);
          setArtifacts((current) => current.filter((candidate) => candidate.id !== item.id));
          setUploadStates((current) => {
            const next = { ...current };
            delete next[key];
            return next;
          });
        } catch (error) {
          const message = error instanceof Error ? error.message : "Upload failed. Check the local API and retry.";
          setUploadStates((current) => ({ ...current, [key]: { status: "failed", progress: 0, error: message } }));
          throw new Error(`Could not upload ${item.file.name}: ${message}`);
        }
      }

      savedArtifacts = await listArtifacts(targetCaseId);
      const diagnosticArtifacts = savedArtifacts.filter((artifact) => (artifact.kind === "heap_dump" || artifact.kind === "thread_dump") && (newlyUploadedArtifactIds.length === 0 || newlyUploadedArtifactIds.includes(artifact.id)));
      if (!diagnosticArtifacts.length) {
        throw new Error("No supported heap or thread dump was identified among the saved files. Check the file extensions and try again.");
      }

      setPersistedArtifacts(savedArtifacts);
      setPersistenceMessage("Evidence saved. Starting the matching diagnostic plugin…");
      const result = await prepareDiagnostics(targetCaseId, problem, diagnosticArtifacts.map((artifact) => artifact.id));
      setPersistenceMessage(`Diagnostic workflow finished with ${result.preparations.length} plugin result(s). Review each status and warning to confirm whether MAT actually ran.`);
      return result;
    } catch (error) {
      const message = error instanceof Error ? error.message : "Could not prepare diagnostic files. Check the local API and retry.";
      setPersistenceMessage(message);
      throw error;
    } finally {
      setPersistenceBusy(false);
    }
  };

return (
    <main className="app-shell">
      <aside className={`sidebar ${showMenu ? "sidebar-open" : ""}`}>
        <a className="brand" href="#" aria-label="Middleware Atlas home">
          <span className="brand-mark"><Layers3 size={21} strokeWidth={2.2} /></span>
          <span className="brand-copy">
            <strong>middleware<span>atlas</span></strong>
            <small>SUPPORT ENGINEERING</small>
          </span>
          <button className="mobile-close icon-button" onClick={() => setShowMenu(false)} aria-label="Close navigation"><X size={18} /></button>
        </a>

        <div className="workspace-switcher">
          <div className="workspace-avatar">M</div>
          <div className="workspace-copy"><strong>My workspace</strong><small>Local environment</small></div>
          <ChevronDown size={15} className="muted" />
        </div>

        <div className="nav-label">WORKSPACE</div>
        <nav className="main-nav" aria-label="Main navigation">
          <a className="nav-item active" href="#intake"><FolderOpen size={17} /><span>Case workspace</span><span className="nav-count">01</span></a>
          <a className="nav-item" href="#artifacts"><HardDrive size={17} /><span>Artifacts</span></a>
          <a className="nav-item" href="#analysis"><Activity size={17} /><span>Analysis runs</span></a>
          <a className="nav-item" href="#evidence"><Network size={17} /><span>Evidence explorer</span><span className="soon-dot" /></a>
        </nav>

        <div className="nav-label nav-label-spaced">DIAGNOSTICS</div>
        <nav className="main-nav" aria-label="Diagnostic tools">
          <a className="nav-item" href="#kafka"><Boxes size={17} /><span>Kafka &amp; streaming</span></a>
          <a className="nav-item" href="#jvm"><Atom size={17} /><span>JVM diagnostics</span></a>
          <a className="nav-item" href="#platform"><Server size={17} /><span>Platform &amp; cluster</span></a>
        </nav>

        <div className="sidebar-bottom">
          <div className="local-card">
            <div className="local-icon"><ShieldCheck size={17} /></div>
            <div><strong>Local-first workspace</strong><p>Uploads are stored on this machine through the Rust API.</p></div>
          </div>
          <a className="nav-item" href="#help"><CircleHelp size={17} /><span>Help &amp; documentation</span></a>
          <div className="profile-row">
            <div className="profile-avatar">SE</div>
            <div className="profile-copy"><strong>Support engineer</strong><small>Local session</small></div>
            <button className="icon-button" title="Workspace options" onClick={() => setShowMenu(!showMenu)}><Command size={16} /></button>
          </div>
        </div>
      </aside>

      {showMenu && <button className="mobile-scrim" aria-label="Close menu" onClick={() => setShowMenu(false)} />}

      <section className="main-panel">
        <header className="topbar">
          <div className="breadcrumbs">
            <button className="mobile-menu icon-button" onClick={() => setShowMenu(true)} aria-label="Open navigation"><Menu size={19} /></button>
            <span>Workspace</span><span className="crumb-slash">/</span><strong>Case intake</strong>
          </div>
          <div className="topbar-actions">
            <span className="environment"><span className="status-dot" /> Local environment</span>
            <span className="top-divider" />
            <button className="icon-button help-button" title="Help"><LifeBuoy size={18} /></button>
            <div className="mini-avatar">SE</div>
          </div>
        </header>

        <div className="content" id="intake">
          <div className="page-intro">
            <div>
              <div className="eyebrow"><span className="eyebrow-line" /> INCIDENT WORKSPACE <span className="eyebrow-dot">·</span> STEP 01 OF 03</div>
              <h1>Bring the evidence.<br /><span>Find the answers.</span></h1>
              <p className="intro-copy">Start with your diagnostic artifacts. Atlas will help organize the evidence for a focused investigation.</p>
            </div>
            <div className="intro-illustration" aria-hidden="true">
              <div className="illustration-orbit orbit-one" />
              <div className="illustration-orbit orbit-two" />
              <div className="illustration-core"><Layers3 size={29} /></div>
              <div className="float-node node-a"><FileText size={15} /></div>
              <div className="float-node node-b"><Activity size={15} /></div>
              <div className="float-node node-c"><Network size={15} /></div>
              <div className="illustration-spark spark-a">✳</div>
              <div className="illustration-spark spark-b">·</div>
            </div>
          </div>

          <div className="stepper" aria-label="Case setup steps">
            <div className="step active-step"><span className="step-number">01</span><span>Collect artifacts</span></div>
            <span className="step-connector" />
            <div className="step"><span className="step-number">02</span><span>Define the case</span></div>
            <span className="step-connector" />
            <div className="step"><span className="step-number">03</span><span>Investigate</span></div>
          </div>

          <section className="upload-card">
            <div className="section-heading">
              <div className="section-icon"><CloudUpload size={19} /></div>
              <div className="section-heading-copy">
                <h2>Diagnostic artifacts</h2>
                <p>Bring logs, dumps, reports, and configuration files into one workspace.</p>
              </div>
              <span className="optional-pill">STEP 1</span>
            </div>

            <input
              ref={inputRef}
              className="visually-hidden"
              type="file"
              multiple
              onChange={onInputChange}
              aria-label="Choose diagnostic files"
            />

            <div
              className={`dropzone ${dragging ? "dropzone-active" : ""}`}
              onDragEnter={(event) => { event.preventDefault(); setDragging(true); }}
              onDragOver={(event) => event.preventDefault()}
              onDragLeave={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false);
              }}
              onDrop={onDrop}
            >
              <div className="upload-art">
                <div className="upload-art-back"><FileText size={25} /></div>
                <div className="upload-art-front"><ArrowDownToLine size={22} /></div>
                <span className="upload-art-spark">✳</span>
              </div>
              <h3>{dragging ? "Drop your files here" : "Drop your diagnostic files here"}</h3>
              <p className="drop-description">Drag and drop one or more files, or browse from your computer.</p>
              <button className="button button-primary browse-button" onClick={() => inputRef.current?.click()}>
                <Plus size={16} /> Browse files
              </button>
              <div className="upload-footnote"><ShieldCheck size={13} /> Files upload to local storage when you save the case.</div>
            </div>

            <div className="supported-row">
              <span className="supported-label">COMMON ARTIFACTS</span>
              <div className="artifact-types">
                <span><FileCode2 size={14} /> Logs</span>
                <span><FileSearch size={14} /> JVM dumps</span>
                <span><FileArchive size={14} /> sosreports</span>
                <span><Network size={14} /> Cluster data</span>
              </div>
            </div>
          </section>

          <section className="files-section" id="artifacts">
            <div className="files-heading">
              <div>
                <h2>Case artifacts <span className="count-pill">{totalArtifactCount}</span></h2>
                <p>Review what you have gathered before moving to case details.</p>
              </div>
              <button className="button button-secondary add-more" onClick={() => inputRef.current?.click()}>
                <Plus size={15} /> Add files
              </button>
            </div>

            {totalArtifactCount === 0 ? (
              <div className="empty-files">
                <div className="empty-icon"><FolderOpen size={21} /></div>
                <div><strong>Your evidence starts here</strong><p>Files you select will appear here, ready for review.</p></div>
              </div>
            ) : (
              <>
                <div className="file-toolbar">
                  <div className="search-field"><Search size={15} /><input aria-label="Search files" placeholder="Search files..." value={query} onChange={(e) => setQuery(e.target.value)} /><kbd>⌘ K</kbd></div>
                  <select aria-label="Filter files by type" className="filter-select" value={filter} onChange={(e) => setFilter(e.target.value)}>
                    <option>All files</option>
                    {categories.map((kind) => <option key={kind}>{kind}</option>)}
                  </select>
                </div>
                <div className="file-list">
                  {visibleFiles.map((item) => {
                    const key = item.id;
                    const upload = uploadStates[key];
                    return (
                      <div className="file-row" key={item.id}>
                        <div className={`file-icon ${kindStyles[item.kind]}`}><FileGlyph kind={item.kind} /></div>
                        <div className="file-info">
                          <strong title={item.file.name}>{item.file.name}</strong>
                          <span>{formatSize(item.file.size)} <span className="meta-separator">·</span> Pending upload</span>
                          {upload?.error && <span role="alert" style={{ color: "#b42318", overflowWrap: "anywhere" }}>{upload.error}</span>}
                          {upload?.status === "uploading" && (
                            <div aria-label={`Upload progress ${upload.progress}%`} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={upload.progress} style={{ height: 4, marginTop: 7, background: "#e6e8f1", borderRadius: 999, overflow: "hidden" }}>
                              <div style={{ width: `${upload.progress}%`, height: "100%", background: "#635bdb", transition: "width .12s linear" }} />
                            </div>
                          )}
                        </div>
                        <span className={`kind-badge ${kindStyles[item.kind]}`}>{item.kind}</span>
                        <span className="file-ready">
                          {upload?.status === "uploading" ? <Upload size={13} /> : upload?.status === "failed" ? <X size={13} /> : <Check size={13} />}
                          {upload?.status === "uploading" ? `${upload.progress}%` : upload?.status === "failed" ? "Retry needed" : "Ready"}
                        </span>
                        <button className="icon-button remove-file" title={`Remove ${item.file.name}`} aria-label={`Remove ${item.file.name}`} onClick={() => removeFile(item.id)}><Trash2 size={16} /></button>
                      </div>
                    );
                  })}
                  {visiblePersistedArtifacts.map((item) => {
                    const kind = displayKindFromApi(item.kind);
                    return (
                      <div className="file-row" key={`stored-${item.id}`}>
                        <div className={`file-icon ${kindStyles[kind]}`}><FileGlyph kind={kind} /></div>
                        <div className="file-info">
                          <strong title={item.filename}>{item.filename}</strong>
                          <span>{formatSize(item.size_bytes)} <span className="meta-separator">·</span> {item.product}</span>
                          <span title={item.sha256} style={{ fontFamily: "ui-monospace, monospace", fontSize: 11 }}>SHA-256 {item.sha256.slice(0, 12)}…</span>
                        </div>
                        <span className={`kind-badge ${kindStyles[kind]}`}>{kind}</span>
                        <span className="file-ready"><Check size={13} /> Stored</span>

                      </div>
                    );
                  })}
                  {visibleFiles.length === 0 && visiblePersistedArtifacts.length === 0 && <div className="no-results">No artifacts match this search.</div>}
                </div>
                <div className="file-summary"><span><Check size={14} /> {totalArtifactCount} artifact{totalArtifactCount === 1 ? "" : "s"} in this case</span><span>{formatSize(totalBytes)} total</span><button className="clear-all" onClick={() => { setArtifacts([]); setUploadStates({}); }}>Clear selections</button></div>
              </>
            )}
          </section>

          <AgentConversation onPrepare={handlePrepareDiagnostics} hasStagedFiles={artifacts.length > 0} onNewInvestigation={() => { setAgentSessionCaseId(null); try { window.sessionStorage.removeItem("atlas.agentSessionCaseId"); window.localStorage.removeItem("atlas.agent.activeCaseId"); } catch { /* storage can be disabled */ } }} />

          <div className="bottom-actions">
            <div className="privacy-note"><ShieldCheck size={16} /><span><strong>Evidence-first by design</strong><br />Large dumps should be analyzed by diagnostic tools, not sent raw to an LLM.</span></div>

      {persistenceMessage && (
        <div role="status" aria-live="polite" style={{ position: "fixed", left: "50%", bottom: 24, transform: "translateX(-50%)", zIndex: 9999, maxWidth: "min(640px, calc(100vw - 32px))", padding: "12px 16px", borderRadius: 12, background: "#202838", color: "white", boxShadow: "0 10px 36px rgba(20,29,48,.22)", fontSize: 13, lineHeight: 1.45 }}>
          {persistenceBusy ? "Saving locally… " : ""}{persistenceMessage}
        </div>
      )}
          </div>

          {notice && (
            <div className="notice" role="status">
              <div className="notice-icon"><Clock3 size={16} /></div>
              <span>{notice}</span>
              <button className="icon-button" onClick={() => setNotice("")} aria-label="Dismiss message"><X size={15} /></button>
            </div>
          )}

          <footer className="page-footer">
            <span>Middleware Atlas <span className="footer-dot">·</span> Built for evidence-led troubleshooting</span>
            <span><span className="footer-status" /> Local workspace</span>
          </footer>
        </div>
      </section>
    </main>
  );
}
