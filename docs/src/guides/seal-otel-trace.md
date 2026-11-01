# Seal an OpenTelemetry trace from any agent

Documented for Akmon `3.0.0`.

If your agent emits OpenTelemetry GenAI traces, Akmon can seal a session from it without touching the agent. This guide covers getting the trace out, sealing it, and deciding how much content to capture.

## 1. Get the trace as OTLP/JSON

Akmon reads one OTLP `ExportTraceServiceRequest` JSON document per file (the body an OTLP/HTTP JSON exporter sends). Common ways to get one:

- **OpenTelemetry Collector.** Add a `file` exporter to the traces pipeline. It writes one JSON document per line; each line is one `ExportTraceServiceRequest`.
- **SDK exporters.** Some SDKs can write OTLP/JSON to a file or stdout directly.
- **Your observability backend**, if it can export a trace in OTLP/JSON.

If your file holds one JSON document per line, put the line you want in its own file first; a multi-line file is refused with `parse_error`. If one document holds spans from several traces, pick one with `--trace-id`.

Supported conventions: the structured form from semantic conventions v1.37 through v1.42, and the legacy v1.36-and-earlier message events. See [akmon otel import](../reference/otel-import.md) for the details and the input rules.

## 2. Decide on content capture

OpenTelemetry instrumentations do not record prompts and responses unless you turn that on, usually with an instrumentation-specific setting such as `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true`. The choice sets the [capture level](../concepts/capture-levels.md):

- Content off: `structural`. The bundle proves which models and tools were called, in what order, and with what outcome, but not what was said.
- Content on: `full` (or `partial` if some calls lack content). The bundle also contains the messages.

Full capture makes stronger evidence and a more sensitive file. If a bundle must leave your organization, plan for [redaction](../reference/redact.md) and for who may read it.

## 3. Seal it

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub   # once
akmon seal trace.json --key signer.pk8 --output audit.akmon
```

`akmon seal` prints the capture level, writes `audit.akmon` and `audit.akmon.pub`, and prints the commands to give your verifier. Add `--proof-dir proof` if the verifier wants to check the signature with OpenSSL alone, and `--operator-key … --operator-id …` to record who approved the session.

To keep the imported session in a specific journal (for example one per project), pass `--journal <DIR>`.

## 4. Check what you sealed

```bash
agef-verify audit.akmon --verify-key audit.akmon.pub --require-signature
```

Read the `capture:` line. If it says `STRUCTURAL` and you expected content, the instrumentation did not record it.

## Automating it

In CI or a collector-side job, seal each trace as it is exported:

```bash
akmon seal "$TRACE" --key "$SIGNER_KEY" --output "evidence/$(basename "$TRACE" .json).akmon" --format json
```

`--format json` prints a single object with `session_id`, `bundle`, `capture_level`, and `verify_command`. Exit codes: `0` sealed, `1` verification failure, `2` bad input, `3` I/O error.

## See also

- [Walkthrough: a metadata-only OTEL trace to an OpenSSL proof](../tutorials/otel-to-openssl-walkthrough.md)
- [akmon seal](../reference/seal.md)
- [Hand evidence to an auditor](./hand-to-auditor.md)
