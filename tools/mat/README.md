# Middleware Atlas MAT container

This directory builds a headless Eclipse Memory Analyzer (MAT) worker from the official Eclipse MAT 1.17.0 stand-alone release and Eclipse Temurin Java 21. The image is built on first analysis if the selected Docker or Podman runtime is available and the configured image is absent.

The Rust API supplies only fixed operation names and validated parameters. The image has no network access at runtime and receives a private per-case analysis workspace. The heap dump is copied into that workspace before MAT starts, so parser activity cannot modify the saved source artifact through a hard link. The copy requires extra free disk space roughly equal to the dump size. The worker runs one-shot jobs and exits.

Supported job operations: `analyze_heap_dump`, `generate_leak_suspects`, `generate_heap_overview`, `get_histogram`, `get_dominator_tree`, `find_paths_to_gc_roots`, and `compare_heap_dumps`.

Build manually from the repository root with either runtime:

```bash
docker build --pull -t middleware-atlas/mat:1.17.0 tools/mat
# or
podman build --pull -t middleware-atlas/mat:1.17.0 tools/mat
```

Do not run arbitrary commands from model output. Raw heap dumps and MAT index/cache files are never sent to an LLM by this adapter. The bounded text/CSV context is sent only when ATLAS_DIFY_SEND_HEAP_CONTEXT=true and server-side Dify credentials are configured.

Upstream documentation: https://help.eclipse.org/latest/topic/org.eclipse.mat.ui.help/tasks/batch.html
