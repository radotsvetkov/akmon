# Test fixtures: OpenTelemetry GenAI traces

## `openai_v2_weather_legacy.otlp.json`

`openai_v2_weather_legacy.otlp.json` is a **representative, illustrative** OpenTelemetry
GenAI trace that models the **default** emission shape of the
[`opentelemetry-instrumentation-openai-v2`](https://pypi.org/project/opentelemetry-instrumentation-openai-v2/)
Python instrumentation (the official OpenTelemetry Python contrib instrumentation for the
OpenAI client). It is a single OTLP/JSON `ExportTraceServiceRequest` with two spans:

- **SPAN 1** — `chat gpt-4o-mini` (kind 3 / CLIENT, root): the provider call, carrying
  structural `gen_ai.*` span attributes (`gen_ai.operation.name=chat`, `gen_ai.system=openai`,
  `gen_ai.request.model`, `gen_ai.response.model`, `gen_ai.response.id`,
  `gen_ai.request.temperature`, `gen_ai.request.max_tokens`, `gen_ai.usage.input_tokens`,
  `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons`) plus the **legacy**
  (`<= v1.36`) message-event form: `gen_ai.system.message`, `gen_ai.user.message`, and
  `gen_ai.choice` span events.
- **SPAN 2** — `execute_tool get_weather` (kind 1 / INTERNAL, child of SPAN 1): the tool
  execution, carrying `gen_ai.operation.name=execute_tool`, `gen_ai.tool.name`, and
  `gen_ai.tool.call.id`.

### It is hand-authored, not a captured session

This fixture is **hand-authored to match the documented emission shape** of that
instrumentation. It is **NOT a verbatim capture of any real production session** and contains
**NO real user data or PII**. The trace/span ids, response id, and tool-call id are synthetic.

### Why it imports as `capture_level=structural`

The message-event **bodies** (the `content` / `message` / `tool_calls` fields) are
**intentionally ABSENT**. This instrumentation does **not** capture message/tool content
unless the operator opts in via `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` (default
**off**). With content capture off, the events are still emitted — proving real legacy
emission — but carry only their structural metadata:

- `gen_ai.system.message` / `gen_ai.user.message`: just the Recommended `gen_ai.system`
  attribute (no `content`).
- `gen_ai.choice`: the Required `index` and `finish_reason`, plus `gen_ai.system` (no nested
  `message` body).

Because no event carries a real body, Akmon imports this trace honestly as
**`capture_level=structural`** (metadata only — message content was not captured by the source
telemetry).

### Correction / honest caveat about `gen_ai.choice`

The OpenTelemetry GenAI **events** specification at v1.36.0 permits `gen_ai.choice` to carry a
`message` map (including `tool_calls` metadata) even when message *content* is off. This fixture
**deliberately omits any `message` body** to model the most honest, fully content-off default.
Do **not** read this fixture as a claim that real emitters never include a `message` map with
tool-call metadata — some do; this one is the conservative content-off case.

### Cosmetic `source_semconv` quirk (disclosed, not fixed)

Akmon's importer records `source_semconv=1.37.0` in the signed session config object for **all**
imports, regardless of the source FORM (this is a hardcoded constant, `SOURCE_SEMCONV`, in
`crates/akmon-otel/src/objects.rs`). This fixture's **source form is the legacy `<= v1.36`
message-event convention**, so the recorded `source_semconv` value does not describe the form.
This is cosmetic and out of scope to change here (it would alter signed config-object bytes,
i.e. substrate). It is disclosed so the value is never mistaken for an accurate form descriptor.

### Grounding sources

- `opentelemetry-instrumentation-openai-v2` (PyPI):
  <https://pypi.org/project/opentelemetry-instrumentation-openai-v2/>
- OpenTelemetry GenAI semantic conventions — events spec v1.36.0:
  <https://github.com/open-telemetry/semantic-conventions/blob/v1.36.0/docs/gen-ai/gen-ai-events.md>
- OpenTelemetry GenAI semantic conventions — current GenAI spans spec:
  <https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-spans/>

### Verified by an automated test

This fixture is driven end-to-end by the integration test
`crates/akmon-cli/tests/e2e_otel_to_openssl_integration.rs`
(`t_e2e_otel_legacy_trace_to_openssl_proof` and the deterministic
`t_e2e_otel_proof_artifacts_byte_identical`).
