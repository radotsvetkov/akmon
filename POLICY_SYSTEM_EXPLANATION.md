# Akmon Policy Evaluation & Permission System

## Overview

The Akmon permission system is a **deny-by-default**, **mode-driven** policy engine that evaluates tool requests before execution. It provides multiple security modes ranging from strict (deny everything) to permissive (auto-approve reads), with full audit logging of all decisions.

---

## Architecture

### Core Components

#### 1. **Permission Types** (`crates/akmon-core/src/permission.rs`)

```rust
pub enum Permission {
    ReadFile { path: PathBuf },
    ListDirectory { path: PathBuf },
    WriteFile { path: PathBuf },
    ExecuteCommand { command: String, cwd: PathBuf },
    NetworkFetch { url: String },
}
```

Each permission variant represents a capability that tools request. Paths are **logical** (relative to sandbox root); the `Sandbox` validates them before any actual filesystem access.

#### 2. **PolicyEngineMode** (`crates/akmon-core/src/policy.rs`)

The engine operates in one of five modes:

```rust
pub enum PolicyEngineMode {
    /// Default: deny all automatic evaluations (safest)
    DenyAll,
    
    /// Require live caller verdict for every request
    Interactive,
    
    /// Auto-approve reads; writes/shell need confirmation if confirm_writes=true
    AutoApproveReads { confirm_writes: bool },
    
    /// Like AutoApproveReads but also auto-approve network fetches
    AutoApproveReadsAndFetch { confirm_writes: bool },
    
    /// Use declarative rules from PolicyConfig (skeleton; rules TBD)
    Configured(PolicyConfig),
}
```

#### 3. **PolicyEngine** (`crates/akmon-core/src/policy.rs`)

The main decision-making component:

```rust
pub struct PolicyEngine {
    mode: PolicyEngineMode,
}

pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: String,
    pub audit: AuditEvent,
}
```

---

## How It Works

### 1. Tool Request → Permission Extraction

When a tool is called, the session extracts concrete permissions from the tool's arguments:

```rust
// From crates/akmon-query/src/session.rs::concrete_permissions()
fn concrete_permissions(
    tool: &dyn Tool,
    name: &str,
    args: &Value,
    sandbox_root: &Path,
) -> Vec<Permission>
```

**Example flow:**
- Tool: `read_file` with `{"path": "README.md"}`
- Extracted permission: `Permission::ReadFile { path: PathBuf::from("README.md") }`

### 2. Policy Evaluation

The engine evaluates each permission through `evaluate_automatic()`:

```rust
pub fn evaluate_automatic(
    &self,
    session_id: &str,
    permission: Permission,
) -> Result<PolicyDecision, PolicyEngineError>
```

**Decision logic by mode:**

| Mode | ReadFile | WriteFile | ExecuteCommand | NetworkFetch |
|------|----------|-----------|----------------|--------------|
| **DenyAll** | ❌ Deny | ❌ Deny | ❌ Deny | ❌ Deny |
| **Interactive** | ❓ Ask | ❓ Ask | ❓ Ask | ❓ Ask |
| **AutoApproveReads** (confirm_writes=false) | ✅ Allow | ❌ Deny | ❌ Deny | ❌ Deny |
| **AutoApproveReads** (confirm_writes=true) | ✅ Allow | ❓ Ask | ❓ Ask | ❌ Deny |
| **AutoApproveReadsAndFetch** (confirm_writes=false) | ✅ Allow | ❌ Deny | ❌ Deny | ✅ Allow |
| **AutoApproveReadsAndFetch** (confirm_writes=true) | ✅ Allow | ❓ Ask | ❓ Ask | ✅ Allow |
| **Configured** | Uses rules | Uses rules | Uses rules | Uses rules |

### 3. Interactive Confirmation (When Required)

If `evaluate_automatic()` returns `Err(PolicyEngineError::InteractiveRequiresCaller)`:

1. Session emits `AgentEvent::ConfirmationRequired` with a user-friendly description
2. Session waits for a `PolicyVerdict` (Allow/Deny) from the caller
3. Session calls `resolve_interactive()` to record the user's decision

```rust
pub fn resolve_interactive(
    &self,
    session_id: &str,
    permission: Permission,
    verdict: PolicyVerdict,
    reason: impl Into<String>,
) -> Result<PolicyDecision, PolicyEngineError>
```

### 4. Audit Logging

Every decision produces an `AuditEvent::PolicyEvaluation`:

```rust
pub enum AuditEvent {
    PolicyEvaluation {
        session_id: String,
        timestamp: DateTime<Utc>,
        permission: Permission,
        verdict: PolicyVerdict,
        reason: String,
    },
    // ... other event types
}
```

Events are:
- Appended to the session's audit log
- Serializable to JSON Lines (JSONL) format
- Safe for logs (no secrets in reason strings)

---

## Integration Flow in AgentSession

### Dispatch Phase

When the model returns a `StopReason::ToolUse`:

```
1. For each tool call:
   a. Find tool by name
   b. Extract concrete_permissions() from arguments
   c. For each permission:
      - Call policy.evaluate_automatic()
      - If InteractiveRequiresCaller:
        * Emit ConfirmationRequired event
        * Wait for PolicyVerdict from caller
        * Call policy.resolve_interactive()
      - If denied: record policy_denial_message, skip execution
      - If allowed: add to approved batch
   
2. Execute approved tools in parallel
   
3. Append tool results to context in request order (not completion order)
```

### Code Location

See `crates/akmon-query/src/session.rs::dispatch_tool_calls_batch()`:

```rust
async fn dispatch_tool_calls_batch(
    &mut self,
    tool_calls: Vec<ModelToolCall>,
    event_tx: &mpsc::Sender<AgentEvent>,
    task: &str,
    interactive_policy_rx: &mut Option<mpsc::Receiver<PolicyVerdict>>,
) -> Result<(), AgentError>
```

---

## Security Guarantees

### Deny-by-Default
- Default mode is `DenyAll` — no permissions are granted without explicit configuration
- Interactive mode requires a live caller to approve each action
- Even in permissive modes (AutoApproveReads), shell commands are never auto-approved

### Sandbox Validation
- All file paths go through `Sandbox::validate_path()` **after** policy approval
- Prevents path traversal attacks (e.g., `../../../etc/passwd`)
- Enforces root directory boundaries

### Audit Trail
- Every policy decision is logged with session ID, timestamp, permission, verdict, and reason
- Audit logs are tamper-evident (append-only, JSONL format)
- Suitable for compliance and forensic analysis

### No Secrets in Logs
- Reason strings contain no API keys, passwords, or sensitive data
- Tool output is sanitized in audit events
- User-facing descriptions are carefully crafted

---

## Modes in Practice

### CLI Mapping

From `crates/akmon-cli/src/main.rs`:

```bash
# Default: deny all
akmon "task"

# Interactive: ask for each action
akmon --interactive "task"

# Auto-approve reads only
akmon --yes "task"

# Auto-approve reads + network fetches
akmon --yes-web "task"

# Auto-approve reads + confirm writes
akmon --yes --confirm-writes "task"
```

### Use Cases

| Scenario | Mode | Reason |
|----------|------|--------|
| Untrusted agent | DenyAll | Maximum safety |
| Developer iteration | Interactive | Full control + audit |
| Read-only analysis | AutoApproveReads | Fast feedback, safe |
| Web scraper | AutoApproveReadsAndFetch | Efficient, still no shell |
| Declarative CI/CD | Configured | Rule-based, repeatable |

---

## Error Handling

### PolicyEngineError

```rust
pub enum PolicyEngineError {
    /// Interactive confirmation required
    InteractiveRequiresCaller,
    
    /// resolve_interactive() called in wrong mode
    NotInteractive,
}
```

**When InteractiveRequiresCaller is raised:**
- The session **does not execute the tool**
- Instead, it emits `ConfirmationRequired` and awaits a verdict
- If no verdict channel is provided, the session fails with `SessionFailed`

**When NotInteractive is raised:**
- The caller tried to resolve a verdict in DenyAll or Configured mode
- This is a programming error, not a user action

---

## Audit Event Structure

### Example: Denied Read

```json
{
  "event_kind": "policy_evaluation",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2024-01-15T10:30:45.123Z",
  "permission": {
    "permission": "write_file",
    "path": "/sensitive/config.txt"
  },
  "verdict": "deny",
  "reason": "denied: PolicyEngineMode::DenyAll"
}
```

### Example: User-Approved Write

```json
{
  "event_kind": "policy_evaluation",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2024-01-15T10:30:46.456Z",
  "permission": {
    "permission": "write_file",
    "path": "output.txt"
  },
  "verdict": "allow",
  "reason": "user approved (stdin)"
}
```

---

## Testing

The policy engine is extensively tested in `crates/akmon-core/src/policy.rs`:

```rust
#[test]
fn deny_all_emits_audit() { ... }

#[test]
fn interactive_automatic_fails() { ... }

#[test]
fn auto_approve_reads_allows_read_and_list() { ... }

#[test]
fn auto_approve_reads_write_needs_caller() { ... }

#[test]
fn auto_approve_reads_and_fetch_allows_network_fetch() { ... }
```

And in `crates/akmon-query/src/session.rs`:

```rust
#[tokio::test]
async fn policy_denies_one_parallel_call_other_still_executes() { ... }

#[tokio::test]
async fn tool_success_appends_tool_role_message() { ... }
```

---

## Future Enhancements

### PolicyConfig Rules (Skeleton)

Currently, `PolicyConfig` is empty and always denies:

```rust
pub struct PolicyConfig {
    // Intentionally empty — declarative rules land in a later slice.
}

impl PolicyConfig {
    pub fn evaluate_permission(&self, _permission: &Permission) -> (PolicyVerdict, String) {
        (PolicyVerdict::Deny, "denied: PolicyConfig defines no matching rules yet (skeleton)".into())
    }
}
```

Future work will add:
- Path-based allowlists (e.g., `src/**`, `!target/**`)
- URL domain allowlists for network fetches
- Command prefix allowlists (e.g., `cargo test`, but not `rm`)
- Time-based grants (e.g., auto-approve for 5 minutes)

---

## Summary

The Akmon permission system provides:

1. **Typed permissions** for five capability categories
2. **Mode-driven decisions** from deny-all to auto-approve
3. **Interactive confirmation** for sensitive operations
4. **Full audit logging** with session tracking
5. **Sandbox validation** for path safety
6. **Parallel execution** with policy enforcement
7. **No secrets in logs** for compliance

This ensures that even if an AI agent is compromised or misbehaves, it cannot exceed the permissions granted by the user, and every action is auditable.
