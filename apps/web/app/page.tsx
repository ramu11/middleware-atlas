"use client";

import {
  Activity, ArrowDownToLine, ArrowRight, Atom, Boxes, Check, ChevronDown,
  CircleHelp, Clock3, CloudUpload, Command, FileArchive, FileCode2,
  FileSearch, FileText, FolderOpen, HardDrive, Layers3, LifeBuoy, Menu,
  MessageSquareText, Network, Plus, Search, Server, ShieldCheck, Trash2,
  Upload, X, Zap,
} from "lucide-react";
import { useCallback, useMemo, useRef, useState, type ChangeEvent, type DragEvent, useEffect } from "react";

type ArtifactKind =
  | "Kafka logs" | "Artemis logs" | "Heap dump" | "Thread dump"
  | "sosreport" | "OpenShift" | "Debezium" | "Archive" | "Other";

type Artifact = {
  id: string;
  file: File;
  kind: ArtifactKind;
};

const kindStyles: Record<ArtifactKind, string> = {
  "Kafka logs": "kind-kafka",
  "Artemis logs": "kind-artemis",
  "Heap dump": "kind-heap",
  "Thread dump": "kind-thread",
  sosreport: "kind-sos",
  OpenShift: "kind-openshift",
  Debezium: "kind-debezium",
  Archive: "kind-archive",
  Other: "kind-other",
};

function classify(name: string): ArtifactKind {
  const n = name.toLowerCase();
  if (n.endsWith(".hprof") || n.endsWith(".phd")) return "Heap dump";
  if (/thread|javacore|\.tdump/.test(n)) return "Thread dump";
  if (/sosreport|sosreport/.test(n)) return "sosreport";
  if (/debezium|connector/.test(n)) return "Debezium";
  if (/artemis|activemq|broker/.test(n)) return "Artemis logs";
  if (/kafka|server\.log|controller\.log/.test(n)) return "Kafka logs";
  if (/openshift|oc-|must-gather|pod|kube/.test(n)) return "OpenShift";
  if (/\.(zip|tar|gz|tgz|xz|bz2|7z)$/.test(n)) return "Archive";
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

import { createCase, updateCase, uploadArtifact } from "@/lib/atlas-api";
export default function Home() {
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [dragging, setDragging] = useState(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("All files");
  const [notice, setNotice] = useState("");
  const [caseName, setCaseName] = useState("New support case");
  const [editingCase, setEditingCase] = useState(false);
  const [showMenu, setShowMenu] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const addFiles = useCallback((incoming: FileList | File[]) => {
    const selected = Array.from(incoming);
    if (!selected.length) return;

    setArtifacts((current) => {
      const known = new Set(
        current.map((a) => `${a.file.name}:${a.file.size}:${a.file.lastModified}`),
      );
      const fresh = selected.filter((file) => {
        const key = `${file.name}:${file.size}:${file.lastModified}`;
        if (known.has(key)) return false;
        known.add(key);
        return true;
      });
      return [
        ...current,
        ...fresh.map((file) => ({
          id: `${file.name}-${file.size}-${file.lastModified}-${Math.random()}`,
          file,
          kind: classify(file.name),
        })),
      ];
    });
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

  const totalBytes = artifacts.reduce((sum, item) => sum + item.file.size, 0);
  const categories = [...new Set(artifacts.map((item) => item.kind))];

    const [persistenceBusy, setPersistenceBusy] = useState(false);
  const [persistenceMessage, setPersistenceMessage] = useState("");
  const [persistedCaseId, setPersistedCaseId] = useState<string | null>(null);
  const [persistedFileKeys, setPersistedFileKeys] = useState<Record<string, string>>({});

  useEffect(() => {
    const onSelected = (event: Event) => {
      const detail = (event as CustomEvent<{ caseId?: string; name?: string }>).detail;
      if (!detail?.caseId) return;
      setPersistedCaseId(detail.caseId);
      try { setPersistedFileKeys(JSON.parse(window.localStorage.getItem(`atlas.uploadedFiles.${detail.caseId}`) ?? "{}") as Record<string, string>); } catch { setPersistedFileKeys({}); }
      if (detail.name) setCaseName(detail.name);
    };
    const onNew = () => {
      setPersistedCaseId(null);
      setPersistedFileKeys({});
      setPersistenceMessage("New case selected. Continue to case details to save it.");
    };
    window.addEventListener("atlas:case-selected", onSelected);
    window.addEventListener("atlas:new-case", onNew);
    return () => {
      window.removeEventListener("atlas:case-selected", onSelected);
      window.removeEventListener("atlas:new-case", onNew);
    };
  }, []);

  const handlePersistCase = async () => {
    if (persistenceBusy) return;
    setPersistenceBusy(true);
    setPersistenceMessage("Saving case and streaming artifacts to local storage…");
    let activeId = persistedCaseId ?? window.localStorage.getItem("atlas.currentCaseId");
    try {
      const selected = Array.isArray(artifacts) ? (artifacts as unknown as Array<File | { file?: File }> ) : [];
      const pendingFiles = selected
        .map((item) => {
          if (typeof File !== "undefined" && item instanceof File) return item;
          if (item && typeof item === "object" && "file" in item && item.file instanceof File) return item.file;
          return null;
        })
        .filter((item): item is File => item !== null);
      const requestedName = String(caseName ?? "").trim() || "Middleware Atlas support case";
      if (!activeId) {
        const created = await createCase(requestedName);
        activeId = created.id;
        setPersistedCaseId(activeId);
        window.localStorage.setItem("atlas.currentCaseId", activeId);
      } else {
        await updateCase(activeId, requestedName);
      }
      let uploaded = { ...persistedFileKeys };
      try { uploaded = { ...uploaded, ...(JSON.parse(window.localStorage.getItem(`atlas.uploadedFiles.${activeId}`) ?? "{}") as Record<string, string>) }; } catch { /* Ignore corrupt retry bookkeeping and continue. */ }
      let savedCount = 0;
      for (const file of pendingFiles) {
        const key = `${file.name}::${file.size}::${file.lastModified}`;
        if (uploaded[key]) continue;
        const saved = await uploadArtifact(activeId, file);
        uploaded[key] = saved.id;
        savedCount += 1;
        window.localStorage.setItem(`atlas.uploadedFiles.${activeId}`, JSON.stringify(uploaded));
        setPersistedFileKeys({ ...uploaded });
        window.dispatchEvent(new CustomEvent("atlas:case-saved", { detail: { caseId: activeId } }));
      }
      window.dispatchEvent(new CustomEvent("atlas:case-saved", { detail: { caseId: activeId } }));
      setPersistenceMessage(savedCount > 0 ? `Saved case and ${savedCount} artifact${savedCount === 1 ? "" : "s"}.` : (pendingFiles.length > 0 ? "Case is up to date; selected files were already saved." : "Case saved. Add diagnostic files when ready."));
    } catch (error) {
      setPersistenceMessage(error instanceof Error ? `Save failed: ${error.message}` : "Save failed. Check the Rust API and retry.");
      if (activeId) window.dispatchEvent(new CustomEvent("atlas:case-saved", { detail: { caseId: activeId } }));
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
            <div><strong>Local-first workspace</strong><p>Files stay in your browser until a backend is connected.</p></div>
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
              <div className="upload-footnote"><ShieldCheck size={13} /> Files are selected locally; no upload occurs yet.</div>
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
                <h2>Selected files <span className="count-pill">{artifacts.length}</span></h2>
                <p>Review what you have gathered before moving to case details.</p>
              </div>
              <button className="button button-secondary add-more" onClick={() => inputRef.current?.click()}>
                <Plus size={15} /> Add files
              </button>
            </div>

            {artifacts.length === 0 ? (
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
                  {visibleFiles.map((item) => (
                    <div className="file-row" key={item.id}>
                      <div className={`file-icon ${kindStyles[item.kind]}`}><FileGlyph kind={item.kind} /></div>
                      <div className="file-info">
                        <strong title={item.file.name}>{item.file.name}</strong>
                        <span>{formatSize(item.file.size)} <span className="meta-separator">·</span> Selected locally</span>
                      </div>
                      <span className={`kind-badge ${kindStyles[item.kind]}`}>{item.kind}</span>
                      <span className="file-ready"><Check size={13} /> Ready</span>
                      <button className="icon-button remove-file" title={`Remove ${item.file.name}`} aria-label={`Remove ${item.file.name}`} onClick={() => removeFile(item.id)}><Trash2 size={16} /></button>
                    </div>
                  ))}
                  {visibleFiles.length === 0 && <div className="no-results">No files match this search.</div>}
                </div>
                <div className="file-summary"><span><Check size={14} /> {artifacts.length} file{artifacts.length === 1 ? "" : "s"} selected</span><span>{formatSize(totalBytes)} total</span><button className="clear-all" onClick={() => setArtifacts([])}>Clear all</button></div>
              </>
            )}
          </section>

          <section className="case-card">
            <div className="case-card-icon"><MessageSquareText size={18} /></div>
            <div className="case-card-content">
              <div className="case-card-title">
                <h2>Give this investigation a name</h2>
                <span className="optional-pill">OPTIONAL</span>
              </div>
              {editingCase ? (
                <input className="case-name-input" value={caseName} autoFocus onChange={(e) => setCaseName(e.target.value)} onBlur={() => setEditingCase(false)} onKeyDown={(e) => { if (e.key === "Enter") setEditingCase(false); }} aria-label="Investigation name" />
              ) : (
                <button className="case-name-button" onClick={() => setEditingCase(true)} title="Click to rename">{caseName} <span>Edit</span></button>
              )}
              <p>A case name makes it easier to keep your investigation organized.</p>
            </div>
            <div className="case-card-decoration"><Zap size={18} /></div>
          </section>

          <div className="bottom-actions">
            <div className="privacy-note"><ShieldCheck size={16} /><span><strong>Evidence-first by design</strong><br />Large dumps should be analyzed by diagnostic tools, not sent raw to an LLM.</span></div>
            <button
              className="button button-primary continue-button"
              onClick={handlePersistCase}
            >
              Continue to case details <ArrowRight size={16} />
            </button>
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
