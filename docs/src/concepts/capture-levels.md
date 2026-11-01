# Capture levels

Documented for Akmon `3.0.0`.

Every Akmon session records how much of the original session it actually contains. The level is part of the signed session record, so it cannot be raised after sealing, and a metadata-only record can never pass for a full recording.

## The three levels

Akmon counts **content slots**: the request and the response of every provider (model) call, and the input and the output of every tool call.

| Level | Meaning | Typical source |
| --- | --- | --- |
| `full` | Every slot holds real captured content. | The bundled reference agent; an OpenTelemetry trace with message content enabled. |
| `partial` | Some slots hold content, some do not. | A trace where content capture was enabled for some calls, or some content was dropped. |
| `structural` | No slot holds content. The record has the shape of the session: which models and tools were called, in what order, when, with what outcome. | An OpenTelemetry trace with default settings (content capture off). |

Message content is opt-in in the OpenTelemetry GenAI conventions, so many production traces are `structural`. That is still useful evidence of what the agent did; it is not evidence of what it said.

Embeddings calls never count as full, because their output is a vector, not message content.

## Where to see it

- `akmon otel import` and `akmon seal` print the level and the slot count, for example `capture level: full (4/4 content slots captured)`.
- `agef-verify` and `akmon bundle verify` show it in the human output (`capture: FULL …`), in JSON (`capture.level`), and in the Markdown report.

## Requiring a level

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature --require-capture full
```

`--require-capture full` fails on `partial` and `structural`. Decide up front which kinds of evidence need full content, and gate on it.

## Full capture is not replay

Replay (`akmon replay`) needs every provider response and tool output, recorded by Akmon itself, so only sessions from the bundled reference agent can be replayed. An imported trace, even at `full`, is an evidence record, not an execution, and replay refuses it.

## See also

- [akmon otel import](../reference/otel-import.md)
- [What verification proves](./verifying-evidence.md)
