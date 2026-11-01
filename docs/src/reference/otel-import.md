# akmon otel import

Documented for Akmon `3.0.0`.

Import one OpenTelemetry GenAI trace (OTLP/JSON) into a new AGEF session in the local journal. [`akmon seal`](./seal.md) runs this step for you; use `otel import` directly when you want to inspect the session before sealing it.

## Usage

```bash
akmon otel import trace.json
akmon otel import trace.json --journal ~/audit/journal
akmon otel import batch.json --trace-id 4bf92f3577b34da6a3ce929d0e0e4736
akmon otel import trace.json --format json
```

In human mode the new session id is printed alone on stdout and the details go to stderr, so this works:

```bash
sid=$(akmon otel import trace.json --journal ./j)
akmon bundle export "$sid" --journal ./j --output "$sid.akmon"
```

## Options

| Option | Meaning |
| --- | --- |
| `--journal <DIR>` | Journal directory. Default: the per-user journal (`$XDG_STATE_HOME/akmon/journal`). |
| `--trace-id <HEX>` | Import only this trace. Required when the file holds spans from more than one trace. |
| `--max-bytes <BYTES>` | Refuse larger trace files. Default `268435456` (256 MiB). |
| `--format human\|json` | Output format. Default `human`. |

## Supported trace forms

- **Structured form**, semantic conventions v1.37 through v1.42 (the `semantic-conventions-genai` repository): `gen_ai.input.messages`, `gen_ai.output.messages`, and `gen_ai.system_instructions` as span attributes, or on a `gen_ai.client.inference.operation.details` event.
- **Legacy message-event form**, v1.36 and earlier: `gen_ai.system.message`, `gen_ai.user.message`, `gen_ai.assistant.message`, `gen_ai.tool.message`, `gen_ai.choice`. Reduced to the same structured content.
- Other current `gen_ai.*` events, such as `gen_ai.evaluation.result`, are recorded as structural metadata.

The importer supports semantic conventions **1.42.0**. The GenAI conventions are still at Development stability upstream, so attribute names can change between releases. The version the producer declared (its schema URL) is recorded as `source_semconv`, or `undeclared`.

## Capture levels

GenAI message content is opt-in in OpenTelemetry, so many traces carry metadata only. The importer counts content slots: the request and response of every provider call, and the input and output of every tool call.

| Level | Meaning |
| --- | --- |
| `full` | Every slot holds real captured content. |
| `partial` | Some slots do. |
| `structural` | None do: the shape of the session (calls, tools, timing, models) without content. |

The level is part of the signed session record. `akmon bundle verify --require-capture full` and `agef-verify --require-capture full` reject anything below `full`. Embeddings calls never count as full, because their output is not message content. See [Capture levels](../concepts/capture-levels.md).

## Input rules

- One trace per import. Use `--trace-id` when the file holds several.
- Byte-identical duplicate spans are dropped; differing spans with the same id are refused.
- GenAI spans must carry valid start and end timestamps.
- A trace with no `chat`, `text_completion`, `generate_content`, `embeddings`, or `execute_tool` span is refused.
- Span status `ERROR`, `error.type`, or an exception event records the call as failed.
- The whole trace is validated before anything is written. A refused trace leaves no session behind.

## JSON output

`--format json` prints a report including `session_id`, `capture_level`, `content_slots_real`, `content_slots_total`, `provider_calls`, `tool_calls`, `failed_calls`, `semconv_version` (the importer's version), `source_semconv`, `source_form`, `trace_id`, `journal_path`, and `next_command`.

Error categories (all exit `2`): `multiple_traces`, `trace_id_not_found`, `duplicate_span_id`, `no_genai_spans`, `invalid_timestamp`, `trace_too_large`, `legacy_semconv_unsupported`. I/O and journal errors exit `3`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Trace imported. |
| `2` | Input refused (unparseable, too large, unrecognized legacy event, several traces without `--trace-id`, duplicate span ids, no GenAI spans, invalid timestamps, empty trace). |
| `3` | I/O or environment error. |

## See also

- [Seal an OpenTelemetry trace](../guides/seal-otel-trace.md)
- [akmon seal](./seal.md)
- [akmon bundle export](./bundle-export.md)
