# Diagnostic Tool Contract

This document defines the intended interface for pluggable diagnostic tools.

## Requirements

Every tool adapter must provide:

- Stable ID and version.
- Description and supported operations.
- Typed, validated input.
- Structured output and evidence references.
- Authorization and case scoping.
- Timeout and resource requirements.
- Consistent status/error reporting.
- Audit events.

## Conceptual request

    {
      "tool_id": "mat.heap.leak_suspects",
      "version": "1",
      "case_id": "case-internal-id",
      "artifact_id": "artifact-internal-id",
      "parameters": {}
    }

## Conceptual response

    {
      "status": "succeeded",
      "summary": "Leak Suspects report generated",
      "evidence": [
        {
          "artifact_id": "artifact-internal-id",
          "location": "Leak Suspects section",
          "excerpt": "Bounded, sanitized excerpt"
        }
      ],
      "artifacts": [],
      "warnings": []
    }

These are design examples only. The contract and API routes must be
implemented and versioned before they can be used.

## Security rules

- The model must not choose arbitrary filesystem paths.
- Resolve artifact IDs only after authorization.
- Do not accept arbitrary shell commands.
- Validate all parameters against the tool schema.
- Enforce resource quotas and execution timeouts.
- Redact secrets and sensitive values where appropriate.
- Never treat generated RCA as automatically confirmed.
