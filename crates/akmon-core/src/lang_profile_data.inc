// Included by `lang_profile.rs` — language + framework + DB intelligence.

// ——— Rust language ———
static R_C: &[&str] = &[
    "Use thiserror for library error types, anyhow for binary/app error types — never Box<dyn Error> in production",
    "Never use .unwrap() or .expect() outside tests and prototypes — use ? for propagation",
    "Prefer borrowing (&T, &mut T) over cloning — clone only when ownership transfer is semantically correct",
    "Use tokio::task::spawn_blocking for CPU-bound work called from async context",
    "All public items require rustdoc comments — cargo doc must compile clean",
    "Use newtype pattern to prevent mixing semantically different values: UserId(u64) not raw u64",
    "Prefer iterator chains over explicit for loops for data transformation",
    "Implement Display for user-facing types, derive Debug for internal types",
    "Use #[non_exhaustive] on public enums for forward compatibility",
    "Avoid Arc<Mutex<T>> in hot paths — prefer message passing with tokio channels",
    "Run cargo clippy -- -D warnings in CI — no exceptions",
    "Use cargo-deny for dependency auditing",
];
static R_M: &[&str] = &[
    "Cloning to avoid borrow checker instead of restructuring lifetimes",
    "Blocking calls inside async functions without spawn_blocking",
    "String allocations in hot paths — use Cow<str> or &str",
    "Panicking in library code — return Result always",
    "Missing Send + Sync bounds on async trait objects",
    "Overusing Arc when ownership could be transferred",
];
static LP_RUST: LangProfile = LangProfile {
    language: Language::Rust,
    display_name: "Rust",
    conventions: R_C,
    common_mistakes: R_M,
    testing_approach: "#[cfg(test)] inline modules for unit tests. tests/ directory for integration. proptest for property-based testing. criterion for benchmarks.",
    error_handling: "Result<T, E> for all fallible ops. ? for propagation. thiserror for typed library errors. anyhow for application code. Never panic in library code.",
    toolchain: "rustup, cargo, clippy, rustfmt, cargo-audit, cargo-deny, cargo-expand, cargo-bloat",
    verify_command: "cargo check 2>&1 | head -20",
    skeleton_guidance: "After every file write or edit, run the verify command; if it fails, fix before moving on. Do not start the next file while the previous change still has compile errors.\n\
New modules: declare `pub mod` in the parent first (one line).\n\
New file skeleton: use statements at top; pub struct / enum / trait definitions with empty bodies; impl blocks with fn signatures only and `todo!()` bodies; tests module stub at bottom if applicable.\n\
Then use `edit` to implement each function one at a time.",
    file_size_note: "models.rs: 30–80 lines is normal; handlers.rs: 40–100 lines per handler group; main.rs: keep under 80 lines (delegate to modules); anything over 200 lines: split into submodules.",
};

macro_rules! lp {
    ($lang:ident, $dn:literal, $c:expr, $m:expr, $test:literal, $err:literal, $tool:literal, $verify:expr, $skel:expr, $fs:expr) => {
        LangProfile {
            language: Language::$lang,
            display_name: $dn,
            conventions: $c,
            common_mistakes: $m,
            testing_approach: $test,
            error_handling: $err,
            toolchain: $tool,
            verify_command: $verify,
            skeleton_guidance: $skel,
            file_size_note: $fs,
        }
    };
}

static PY_C: &[&str] = &[
    "All function signatures require type hints — from __future__ import annotations at top of files",
    "Use pyproject.toml not setup.py — PEP 517/518 is the standard",
    "Pydantic v2 for data validation — not raw dicts for contracts",
    "Use pathlib.Path not os.path for filesystem work",
    "f-strings for formatting — not .format or %",
    "Context managers (with) for all resource lifetime management",
    "Never bare except — specify exception types",
    "Async: asyncio.gather for concurrent I/O — do not block the event loop",
    "Virtual environments always; uv for fast dependency management",
    "__all__ to define public API",
];
static PY_M: &[&str] = &[
    "Mutable default arguments",
    "Catching broad Exception",
    "Global mutable module state",
    "Mixing sync I/O inside async tasks",
];
static LP_PYTHON: LangProfile = lp!(
    Python,
    "Python",
    PY_C,
    PY_M,
    "pytest with fixtures; parametrize; mock at boundaries; hypothesis for properties.",
    "Custom exception hierarchies; never suppress silently.",
    "uv, ruff, mypy, pytest, pre-commit, pyright",
    "python -m py_compile {file} && echo OK",
    "New module skeleton: module docstring; from __future__ import annotations; imports grouped (stdlib, third-party, local); type aliases; class/function stubs with pass bodies and type hints; if __name__ == \"__main__\": when a script.\n\
Then `edit` to implement each class or function. In verify_command, replace {file} with the path you just changed.\n\
After adding dependencies: pip install -r requirements.txt (or uv sync / poetry install per project).",
    "models.py: 30–80 lines per model group; routes.py: 20–40 lines per route group — split by domain; never one routes file over 200 lines."
);

static TS_C: &[&str] = &[
    "strict: true and noUncheckedIndexedAccess in tsconfig",
    "Never any — use unknown with narrowing",
    "interface for object shapes; type for unions and mapped types",
    "Always handle promise rejections — no floating promises",
    "Path aliases in tsconfig over ../../../ imports",
    "Discriminated unions for state machines",
    "ReadonlyArray for immutable collections",
    "Zod at API trust boundaries",
    "satisfies for type-checked object literals",
];
static TS_M: &[&str] = &[
    "Type assertions (as) hiding errors",
    "Missing await",
    "Ignoring strictNullChecks",
];
static LP_TYPESCRIPT: LangProfile = lp!(
    TypeScript,
    "TypeScript",
    TS_C,
    TS_M,
    "Vitest; Playwright; vi.mock; MSW for HTTP.",
    "Result-style domain errors; typed catch; custom Error subclasses.",
    "tsx, tsc, eslint, prettier, vitest, tsup",
    "tsc --noEmit 2>&1 | head -20",
    "React component: imports; interface Props { ... }; export default function ComponentName() { return <div /> }; then `edit` for state, effects, handlers, JSX.\n\
Next.js App Router page: import type { Metadata } from 'next'; export const metadata: Metadata = { title: '...' }; export default function Page() { return <main /> }; then `edit` for components and data fetching.\n\
API route / server file: imports; type definitions; handler stubs; exports.\n\
Never use any — find the correct type. After new packages: npm install (or pnpm/yarn).",
    "React component: 50–120 lines is normal; Next.js page: 40–100 (delegate to components); utility modules: keep under ~80 lines; CSS module: 50–150 lines per component."
);

static JS_C: &[&str] = &[
    "ESModules everywhere in new code",
    "Optional chaining ?. and ?? over manual null checks",
    "const / let — never var",
    "async/await over raw promise chains",
    "Array methods over imperative loops where clearer",
    "JSDoc for public APIs when not using TypeScript",
];
static JS_M: &[&str] = &[
    "== instead of ===",
    "Unhandled promise rejections",
    "Mutating function parameters",
];
static LP_JAVASCRIPT: LangProfile = lp!(
    JavaScript,
    "JavaScript",
    JS_C,
    JS_M,
    "Vitest or Jest; Testing Library; Playwright.",
    "try/catch; global rejection handlers; never swallow errors.",
    "node, pnpm/npm, eslint, prettier, vite",
    "node --check {file} 2>&1 (when applicable for the file you changed); otherwise npm run build or npm test if defined.",
    "Same structural skeletons as TypeScript but with JSDoc where types are needed. After new packages: npm install (or pnpm/yarn). Prefer small modules; split files before they exceed ~200 lines.",
    "Keep modules focused; utility files under ~80 lines when possible; split large route or component files by feature."
);

static GO_C: &[&str] = &[
    "Errors are values — check every error return",
    "Accept interfaces, return structs",
    "context.Context as first parameter for I/O functions",
    "Table-driven tests with t.Run",
    "Goroutines: WaitGroup, errgroup, or channels for lifecycle",
    "defer for cleanup",
    "fmt.Errorf with %w for wrapping",
    "Prefer structs over maps for structured data",
];
static GO_M: &[&str] = &[
    "Goroutine leaks",
    "Ignored errors",
    "Large structs passed by value in hot paths",
];
static LP_GO: LangProfile = lp!(
    Go,
    "Go",
    GO_C,
    GO_M,
    "testing + testify; httptest for handlers.",
    "(T, error) returns; errors.Is/As; sentinel errors when appropriate.",
    "go, golangci-lint, govulncheck, air, wire",
    "go build ./... 2>&1 | head -20",
    "package declaration; import block (blank imports for side effects here); type declarations; function signatures with // TODO: implement bodies.\n\
Then `edit` each function. After new modules: go mod tidy.",
    "handler file: 60–150 lines per domain; model file: 20–60 lines; main.go: under 80 lines (wiring only, not business logic)."
);

static JV_C: &[&str] = &[
    "Records for immutable DTOs (Java 16+)",
    "Sealed classes for variants (Java 17+)",
    "Pattern matching in switch (Java 21+)",
    "Stream API for collection transforms",
    "Constructor injection — not field injection",
    "Optional<T> from public APIs",
    "Virtual threads (Java 21+) for concurrent I/O",
    "try-with-resources for closable resources",
];
static JV_M: &[&str] = &[
    "Checked exceptions in lambdas",
    "Mutable domain entities without discipline",
    "String concatenation in loops",
    "resource leaks",
];
static LP_JAVA: LangProfile = lp!(
    Java,
    "Java",
    JV_C,
    JV_M,
    "JUnit 5, AssertJ, Mockito, Testcontainers, ArchUnit.",
    "Unchecked for programming bugs; Optional/Result types for expected domain failures.",
    "Maven/Gradle, SpotBugs, Checkstyle, JaCoCo",
    "./mvnw compile -q 2>&1 | tail -5 OR ./gradlew compileJava -q 2>&1 | tail -5 (use the build tool present in the repo)",
    "@RestController + @RequestMapping skeleton class; then `edit` to add constructor injection and endpoint methods one at a time.",
    "Controllers stay thin; split by resource; service layer for logic — avoid single classes over ~200 lines."
);

static CS_C: &[&str] = &[
    "#nullable enable; records for immutable DTOs",
    "async/await — never block on .Result",
    "Pattern matching / switch expressions",
    "LINQ method syntax",
    "required modifier; init-only properties",
    "IAsyncEnumerable for async streams",
    "using for IDisposable",
];
static CS_M: &[&str] = &[
    "async void outside event handlers",
    "undisposed IDisposable",
    "deferred LINQ executed multiple times",
];
static LP_CSHARP: LangProfile = lp!(
    CSharp,
    "C#",
    CS_C,
    CS_M,
    "xUnit, FluentAssertions, Moq/NSubstitute, WebApplicationFactory.",
    "Result pattern / ProblemDetails; exceptions only for exceptional paths.",
    "dotnet, analyzers, StyleCop, dotnet-format, BenchmarkDotNet",
    "dotnet build -q --no-restore 2>&1 | tail -15",
    "Program.cs / minimal API or controller skeleton first; expand with `edit` method by method.",
    "Split large Program/Startup or controllers by feature; keep files under ~200 lines where practical."
);

static EX_C: &[&str] = &[
    "Pattern match in function heads",
    "with for multi-step {:ok,_} pipelines",
    "Enum/Stream over ad-hoc recursion",
    "Processes for state — not globals",
    "Supervisor trees for fault tolerance",
    "@spec on public functions",
    "Oban for background jobs",
];
static EX_M: &[&str] = &[
    "Business logic in Phoenix controllers",
    "Ignoring {:error,_} returns",
    "critical processes unsupervised",
];
static LP_ELIXIR: LangProfile = lp!(
    Elixir,
    "Elixir",
    EX_C,
    EX_M,
    "ExUnit, Mox, Bypass, DocTests.",
    "{:ok,_} / {:error,_}; supervisors; rescue at boundaries.",
    "mix, credo, dialyxir, excoveralls, ex_doc",
    "mix compile 2>&1 | head -20",
    "defmodule MyApp.ModuleName do — imports/aliases; function stubs returning :ok or placeholder; then `edit` each function.",
    "Contexts and small modules; prefer compilation checks after each file; split modules before they grow past ~200 lines."
);

static RB_C: &[&str] = &[
    "Convention over configuration (Rails)",
    "Fat models, thin controllers",
    "Concerns for shared behavior",
    "Symbols for hash keys",
    "frozen_string_literal: true",
    "Modules for namespacing — shallow inheritance",
];
static RB_M: &[&str] = &[
    "N+1 queries",
    "Callbacks replacing domain services",
    "missing indexes",
];
static LP_RUBY: LangProfile = lp!(
    Ruby,
    "Ruby",
    RB_C,
    RB_M,
    "RSpec, FactoryBot, Capybara, VCR, SimpleCov.",
    "raise/rescue specific exceptions.",
    "bundler, rubocop, brakeman, sorbet",
    "bundle exec ruby -c {file} 2>&1 (syntax check) or bin/rubocop {file} if configured",
    "Class/module skeleton with empty methods and `end`; fill with `edit` incrementally. After Gemfile changes: bundle install.",
    "Keep concerns and service objects small; split fat models/controllers before ~200 lines."
);

static SW_C: &[&str] = &[
    "Value types (struct/enum) by default",
    "Protocol-oriented design",
    "async/await over completion handlers",
    "Actors for shared mutable state",
    "Codable for serialization",
    "No force unwrap in production — guard/if let",
    "Sendable for concurrency boundaries",
];
static SW_M: &[&str] = &[
    "Retain cycles in closures",
    "Main-thread blocking I/O",
    "Massive view controllers",
];
static LP_SWIFT: LangProfile = lp!(
    Swift,
    "Swift",
    SW_C,
    SW_M,
    "XCTest, XCUITest, Swift Testing, snapshot tests.",
    "throws/try; Result; LocalizedError — no try! in prod.",
    "xcodebuild, swift-format, SwiftLint, SPM, Instruments",
    "swift build 2>&1 | head -20",
    "SwiftUI: import SwiftUI; struct ViewName: View { var body: some View { EmptyView() } }; #Preview { ViewName() }; then `edit` for @State and body.\n\
(Without Package.swift, use xcodebuild or the workflow from README.)",
    "SwiftUI View: 60–150 lines; ViewModel: 80–180; Model: 30–80 — extract subviews when larger."
);

static KT_C: &[&str] = &[
    "data / sealed classes for models",
    "Coroutines + Flow",
    "Extension functions with intent",
    "Null safety — avoid !!",
    "val over var",
    "Jetpack Compose on Android when appropriate",
];
static KT_M: &[&str] = &[
    "!! in production",
    "Blocking dispatchers",
    "Flow errors not handled",
];
static LP_KOTLIN: LangProfile = lp!(
    Kotlin,
    "Kotlin",
    KT_C,
    KT_M,
    "JUnit 5 + Kotest, MockK, Turbine, Compose UI tests.",
    "Result types; sealed errors; coroutine exception handlers.",
    "Gradle Kotlin DSL, ktlint, detekt, kover",
    "./gradlew compileDebugKotlin 2>&1 | tail -10",
    "Composable: @Composable fun ScreenName(/* params */) { } then `edit` for UI tree and state.",
    "Split large Composables; keep files cohesive — extract when approaching ~200 lines of UI logic."
);

static DA_C: &[&str] = &[
    "Widget composition over inheritance",
    "const constructors wherever possible",
    "Bloc/Riverpod for non-trivial state",
    "Isolates for CPU-heavy work",
    "go_router for navigation",
    "Stable keys on list children",
];
static DA_M: &[&str] = &[
    "setState storms",
    "work on main isolate",
    "undisposed streams/controllers",
];
static LP_DART: LangProfile = lp!(
    Dart,
    "Dart / Flutter",
    DA_C,
    DA_M,
    "flutter_test, bloc_test, integration_test, golden tests.",
    "Typed failures; FlutterError hooks for framework errors.",
    "flutter, dart analyze, dart format, melos",
    "flutter analyze 2>&1 | head -20",
    "StatefulWidget: StatefulWidget + State class with build returning Placeholder(); StatelessWidget: build => Placeholder(); then `edit` for real UI.\n\
Fix analyzer issues before adding more code.",
    "StatefulWidget file: 80–200 lines is normal; screen: 100–250; service/repository: 50–150; over 250: extract sub-widgets."
);

static CPP_C: &[&str] = &[
    "RAII; smart pointers",
    "const correctness",
    "Prefer STL algorithms",
    "Rule of zero/five where applicable",
];
static CPP_M: &[&str] = &[
    "raw owning pointers",
    "data races across threads",
];
static LP_CPP: LangProfile = lp!(
    Cpp,
    "C++",
    CPP_C,
    CPP_M,
    "GoogleTest/Catch2; sanitizers in CI.",
    "Project error model: exceptions or expected-like types — be consistent.",
    "cmake, ninja, clang-tidy",
    "cmake --build build 2>&1 | head -20 (adjust build dir to match the project)",
    "Headers with declarations; .cpp with empty or stub implementations; compile after each file.",
    "Split translation units before files exceed ~300 lines; keep public headers minimal."
);

static ZG_C: &[&str] = &[
    "explicit allocators",
    "errors as values (error unions)",
    "comptime for generics",
];
static ZG_M: &[&str] = &[
    "ignoring error unions",
    "UB from unchecked bounds",
];
static LP_ZIG: LangProfile = lp!(
    Zig,
    "Zig",
    ZG_C,
    ZG_M,
    "zig test; fuzz where valuable.",
    "errdefer; handle error unions at boundaries.",
    "zig, zig fmt",
    "zig build 2>&1 | head -20",
    "pub fn declarations with unimplemented or TODO bodies; compile; then `edit` each function.",
    "Keep files focused; split when a file grows past ~200–300 lines."
);

static UN_C: &[&str] = &[
    "Follow this repository's existing conventions and linters",
    "Prefer small, reviewable changes",
    "Do not silence errors without justification",
];
static UN_M: &[&str] = &[
    "Guessing stack without reading manifests",
];
static LP_UNKNOWN: LangProfile = lp!(
    Unknown,
    "Unknown",
    UN_C,
    UN_M,
    "Use the project's native test commands (see README).",
    "Match the dominant language's idiomatic error model once detected.",
    "Read README, CI, and config files before refactoring.",
    "Run the check command documented in README or CI after each edit (no generic command detected).",
    "Start from the smallest skeleton the repo already uses (mirror an adjacent file); expand with small edits.",
    "Keep new files small until the stack is confirmed; prefer splitting early."
);

/// Returns the static [`LangProfile`] for a [`Language`].
pub fn lang_profile(language: Language) -> &'static LangProfile {
    match language {
        Language::Rust => &LP_RUST,
        Language::Python => &LP_PYTHON,
        Language::TypeScript => &LP_TYPESCRIPT,
        Language::JavaScript => &LP_JAVASCRIPT,
        Language::Go => &LP_GO,
        Language::Java => &LP_JAVA,
        Language::CSharp => &LP_CSHARP,
        Language::Elixir => &LP_ELIXIR,
        Language::Ruby => &LP_RUBY,
        Language::Swift => &LP_SWIFT,
        Language::Kotlin => &LP_KOTLIN,
        Language::Dart => &LP_DART,
        Language::Cpp => &LP_CPP,
        Language::Zig => &LP_ZIG,
        Language::Unknown => &LP_UNKNOWN,
    }
}

// ——— Framework slice groups ———
static RUST_HTTP_C: &[&str] = &[
    "Thin handlers — delegate to a service layer",
    "Shared state behind Arc / framework state types",
    "Typed error responses — no panics in request path",
    "Middleware ordering: auth, logging, timeouts before business logic",
    "Validate input at the edge (extractors / guards)",
];
static RUST_HTTP_M: &[&str] = &[
    "Blocking I/O inside async handlers",
    "Business rules copy-pasted into handlers",
    "Cloning large app state per request",
];
static RUST_CLI_C: &[&str] = &[
    "Derive-based CLI definitions where possible",
    "Consistent help and long_about for complex subcommands",
    "Validation via clap or typed parsing — not manual panic",
];
static RUST_CLI_M: &[&str] = &[
    "Undocumented flags",
    "Panicking on bad user input",
];
static RUST_TUI_C: &[&str] = &[
    "Separate model from view — one app state struct",
    "Event loop: input → update → draw",
    "Layout with Constraint::Min/Max/Percentage",
    "Non-blocking async bridge to the UI thread",
];
static RUST_TUI_M: &[&str] = &[
    "Business logic inside render",
    "Blocking the tick loop",
];
static RUST_ASYNC_C: &[&str] = &[
    "tokio/async-std: pick one primary runtime per binary",
    "spawn_blocking for CPU-heavy sections",
    "timeouts and cancellation on external I/O",
];
static RUST_ASYNC_M: &[&str] = &[
    "blocking_sleep or std::mutex in async contexts",
];
static RCPU_C: &[&str] = &[
    "Use rayon for data-parallel CPU — not as an async replacement",
    "Chunk work to avoid oversubscription",
];
static RCPU_M: &[&str] = &[
    "Nested parallelism without control",
];
static RUST_ORM_C: &[&str] = &[
    "Migrations tracked in VCS",
    "Bound parameters — never string-concat SQL",
    "Connection pool sizing for production loads",
];
static RUST_ORM_M: &[&str] = &[
    "N+1 queries without batching",
];
static RUST_APP_C: &[&str] = &[
    "Clear separation: command / IPC surface vs domain logic",
    "Minimize trust of UI/webview input",
    "Feature-gate platform-specific code",
];
static RUST_APP_M: &[&str] = &[
    "God objects spanning UI and domain",
];
static PY_WEB_C: &[&str] = &[
    "Pydantic models for request/response shapes",
    "Dependency injection via FastAPI Depends or Django settings patterns",
    "async def for I/O routes when using ASGI stacks",
    "Avoid sync ORM calls inside async handlers without executor",
];
static PY_WEB_M: &[&str] = &[
    "Global mutable singletons for DB",
    "Leaking internal fields in JSON responses",
];
static PY_ML_C: &[&str] = &[
    "Pin dependency versions for reproducible training/inference",
    "Log experiment IDs and data hashes",
    "Validate tensor shapes and devices explicitly",
];
static PY_ML_M: &[&str] = &[
    "Silent dtype/device mismatches",
];
static PY_JOB_C: &[&str] = &[
    "Idempotent Celery tasks with explicit retries",
    "Visibility timeouts aligned with runtime",
];
static PY_JOB_M: &[&str] = &[
    "Infinite retry loops",
];
static JS_FE_C: &[&str] = &[
    "Server components / SSR first when using Next.js",
    "Colocate feature modules; avoid deep prop drilling",
    "TanStack Query (or similar) for remote server state",
];
static JS_FE_M: &[&str] = &[
    "useEffect as a data loader",
    "Giant client bundles importing server-only code",
];
static JS_BE_C: &[&str] = &[
    "DTO validation on every ingress",
    "Centralized error middleware",
    "Structured logging with request ids",
];
static JS_BE_M: &[&str] = &[
    "Fat controllers / route handlers",
];
static JS_DATA_C: &[&str] = &[
    "Schema-first migrations",
    "Avoid implicit N+1 — use includes/joins or batch loaders",
];
static JS_DATA_M: &[&str] = &[
    "Leaking raw DB rows to clients",
];
static GO_HTTP_C: &[&str] = &[
    "middleware chains: recover, log, auth, then handlers",
    "bind validation with explicit 400 responses",
    "context deadlines on outbound calls",
];
static GO_HTTP_M: &[&str] = &[
    "Handler business logic without tests",
];
static JAVA_FW_C: &[&str] = &[
    "Constructor injection; DTOs at API edges",
    "@Transactional on service operations spanning repos",
    "Schema migrations via Flyway/Liquibase",
];
static JAVA_FW_M: &[&str] = &[
    "Exposing JPA entities directly via REST",
];
static CS_WEB_C: &[&str] = &[
    "Minimal APIs or controllers — keep symmetry",
    "ProblemDetails for HTTP errors",
    "Async all the way — no blocking waits",
];
static CS_WEB_M: &[&str] = &[
    "DbContext threading mistakes",
];
static EX_PHX_C: &[&str] = &[
    "Contexts as the domain API — controllers call contexts",
    "Changesets validate all writes",
    "LiveView: minimal assigns; keyed collections",
];
static EX_PHX_M: &[&str] = &[
    "SQL scattered across controllers",
];
static IOS_UI_C: &[&str] = &[
    "Small, composable views",
    "Environment / dependency injection for services",
    "NavigationStack on modern iOS",
];
static IOS_UI_M: &[&str] = &[
    "Heavy work in body / view updates",
];
static AND_CMP_C: &[&str] = &[
    "State hoisting; ViewModel owns mutable state",
    "Stable keys for LazyColumn/Row",
    "Side effects via LaunchedEffect/DisposableEffect appropriately",
];
static AND_CMP_M: &[&str] = &[
    "Recomposition storms from unstable lambdas",
];
static MOB_X_C: &[&str] = &[
    "One navigation system per app (Expo Router / React Navigation)",
    "List virtualization for long feeds",
    "Reanimated (RN) or Flutter isolates for smooth UX",
];
static MOB_X_M: &[&str] = &[
    "Inline styles/objects recreated every render",
];
static API_SPEC_C: &[&str] = &[
    "Proto / OpenAPI / GraphQL schema is the contract — version breaking changes",
    "Pagination and limits on list endpoints",
    "Authn/z at the edge",
];
static API_SPEC_M: &[&str] = &[
    "Unbounded queries",
];

fn make_pf(
    framework: Framework,
    display_name: &'static str,
    conventions: &'static [&'static str],
    common_mistakes: &'static [&'static str],
    patterns: Option<&'static str>,
) -> FrameworkProfile {
    FrameworkProfile {
        framework,
        display_name,
        conventions,
        common_mistakes,
        patterns,
    }
}

fn rust_http_name(f: Framework) -> &'static str {
    match f {
        Framework::Axum => "Axum",
        Framework::Actix => "Actix Web",
        Framework::Rocket => "Rocket",
        Framework::Warp => "Warp",
        _ => "Rust HTTP",
    }
}

/// Guidance for a detected [`Framework`].
pub fn framework_profile(framework: Framework) -> FrameworkProfile {
    match framework {
        k @ (Framework::Axum | Framework::Actix | Framework::Rocket | Framework::Warp) => make_pf(
            k,
            rust_http_name(k),
            RUST_HTTP_C,
            RUST_HTTP_M,
            Some("Router → thin handler → service → data access."),
        ),
        Framework::Clap => make_pf(
            Framework::Clap,
            "Clap",
            RUST_CLI_C,
            RUST_CLI_M,
            Some("Derive Args / Subcommand; groups for exclusivity."),
        ),
        Framework::Ratatui => make_pf(
            Framework::Ratatui,
            "Ratatui",
            RUST_TUI_C,
            RUST_TUI_M,
            Some("Model / view split; crossterm backend."),
        ),
        k @ (Framework::Tokio | Framework::AsyncStd) => make_pf(
            k,
            match k {
                Framework::Tokio => "Tokio",
                Framework::AsyncStd => "async-std",
                _ => "Async",
            },
            RUST_ASYNC_C,
            RUST_ASYNC_M,
            None,
        ),
        Framework::Rayon => make_pf(
            Framework::Rayon,
            "Rayon",
            RCPU_C,
            RCPU_M,
            None,
        ),
        k @ (Framework::Sqlx | Framework::Diesel | Framework::SeaOrm) => make_pf(
            k,
            match k {
                Framework::Sqlx => "SQLx",
                Framework::Diesel => "Diesel",
                Framework::SeaOrm => "SeaORM",
                _ => "ORM",
            },
            RUST_ORM_C,
            RUST_ORM_M,
            None,
        ),
        k @ (Framework::Tauri | Framework::Bevy | Framework::Leptos) => make_pf(
            k,
            match k {
                Framework::Tauri => "Tauri",
                Framework::Bevy => "Bevy",
                Framework::Leptos => "Leptos",
                _ => "Rust app",
            },
            RUST_APP_C,
            RUST_APP_M,
            None,
        ),
        k @ (Framework::FastAPI
        | Framework::Django
        | Framework::Flask
        | Framework::Starlette
        | Framework::Litestar) => make_pf(
            k,
            match k {
                Framework::FastAPI => "FastAPI",
                Framework::Django => "Django",
                Framework::Flask => "Flask",
                Framework::Starlette => "Starlette",
                Framework::Litestar => "Litestar",
                _ => "Python web",
            },
            PY_WEB_C,
            PY_WEB_M,
            Some("Router/module per domain; schemas at the edge."),
        ),
        k @ (Framework::Pandas
        | Framework::Polars
        | Framework::NumPy
        | Framework::Pydantic
        | Framework::PyTorch
        | Framework::TensorFlow
        | Framework::HuggingFace
        | Framework::LangChain
        | Framework::LlamaIndex) => make_pf(
            k,
            match k {
                Framework::Pandas => "Pandas",
                Framework::Polars => "Polars",
                Framework::NumPy => "NumPy",
                Framework::Pydantic => "Pydantic",
                Framework::PyTorch => "PyTorch",
                Framework::TensorFlow => "TensorFlow",
                Framework::HuggingFace => "Hugging Face",
                Framework::LangChain => "LangChain",
                Framework::LlamaIndex => "LlamaIndex",
                _ => "Python ML",
            },
            PY_ML_C,
            PY_ML_M,
            None,
        ),
        Framework::Celery => make_pf(
            Framework::Celery,
            "Celery",
            PY_JOB_C,
            PY_JOB_M,
            None,
        ),
        k @ (Framework::NextJs
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SolidJs
        | Framework::Angular
        | Framework::Astro
        | Framework::Remix) => make_pf(
            k,
            match k {
                Framework::NextJs => "Next.js",
                Framework::React => "React",
                Framework::Vue => "Vue",
                Framework::Svelte => "Svelte",
                Framework::SolidJs => "SolidJS",
                Framework::Angular => "Angular",
                Framework::Astro => "Astro",
                Framework::Remix => "Remix",
                _ => "Web UI",
            },
            JS_FE_C,
            JS_FE_M,
            Some("Feature folders; server/client boundaries explicit."),
        ),
        k @ (Framework::NestJs
        | Framework::Express
        | Framework::Fastify
        | Framework::Hono
        | Framework::ElysiaJs) => make_pf(
            k,
            match k {
                Framework::NestJs => "NestJS",
                Framework::Express => "Express",
                Framework::Fastify => "Fastify",
                Framework::Hono => "Hono",
                Framework::ElysiaJs => "Elysia",
                _ => "Node server",
            },
            JS_BE_C,
            JS_BE_M,
            Some("Module → controller → service layering."),
        ),
        k @ (Framework::Prisma | Framework::Drizzle | Framework::Trpc | Framework::Tanstack) => make_pf(
            k,
            match k {
                Framework::Prisma => "Prisma",
                Framework::Drizzle => "Drizzle",
                Framework::Trpc => "tRPC",
                Framework::Tanstack => "TanStack Query",
                _ => "Data layer",
            },
            JS_DATA_C,
            JS_DATA_M,
            None,
        ),
        k @ (Framework::Gin | Framework::Echo | Framework::Chi | Framework::Fiber | Framework::Gorilla) => {
            make_pf(
                k,
                match k {
                    Framework::Gin => "Gin",
                    Framework::Echo => "Echo",
                    Framework::Chi => "chi",
                    Framework::Fiber => "Fiber",
                    Framework::Gorilla => "Gorilla mux",
                    _ => "Go HTTP",
                },
                GO_HTTP_C,
                GO_HTTP_M,
                None,
            )
        },
        Framework::Cobra => make_pf(
            Framework::Cobra,
            "Cobra",
            RUST_CLI_C,
            RUST_CLI_M,
            Some("Command tree; persistent flags at root."),
        ),
        k @ (Framework::SpringBoot | Framework::Quarkus | Framework::Micronaut | Framework::VertX) => make_pf(
            k,
            match k {
                Framework::SpringBoot => "Spring Boot",
                Framework::Quarkus => "Quarkus",
                Framework::Micronaut => "Micronaut",
                Framework::VertX => "Vert.x",
                _ => "JVM framework",
            },
            JAVA_FW_C,
            JAVA_FW_M,
            Some("Controller → service → repository."),
        ),
        k @ (Framework::AspNetCore | Framework::MauiBlazor) => make_pf(
            k,
            match k {
                Framework::AspNetCore => "ASP.NET Core",
                Framework::MauiBlazor => "MAUI / Blazor",
                _ => ".NET UI",
            },
            CS_WEB_C,
            CS_WEB_M,
            None,
        ),
        k @ (Framework::Phoenix | Framework::LiveView) => make_pf(
            k,
            match k {
                Framework::Phoenix => "Phoenix",
                Framework::LiveView => "Phoenix LiveView",
                _ => "Elixir web",
            },
            EX_PHX_C,
            EX_PHX_M,
            Some("Context-first domain API."),
        ),
        k @ (Framework::SwiftUI | Framework::UIKit | Framework::Combine) => make_pf(
            k,
            match k {
                Framework::SwiftUI => "SwiftUI",
                Framework::UIKit => "UIKit",
                Framework::Combine => "Combine",
                _ => "Apple platforms",
            },
            IOS_UI_C,
            IOS_UI_M,
            Some("MVVM-friendly boundaries."),
        ),
        k @ (Framework::JetpackCompose | Framework::AndroidView) => make_pf(
            k,
            match k {
                Framework::JetpackCompose => "Jetpack Compose",
                Framework::AndroidView => "Android Views",
                _ => "Android UI",
            },
            AND_CMP_C,
            AND_CMP_M,
            Some("UDF: ViewModel → state → composable."),
        ),
        k @ (Framework::FlutterFramework
        | Framework::ReactNative
        | Framework::ExpoFramework
        | Framework::KotlinMultiplatform) => make_pf(
            k,
            match k {
                Framework::FlutterFramework => "Flutter",
                Framework::ReactNative => "React Native",
                Framework::ExpoFramework => "Expo",
                Framework::KotlinMultiplatform => "Kotlin Multiplatform",
                _ => "Cross-platform mobile",
            },
            MOB_X_C,
            MOB_X_M,
            None,
        ),
        k @ (Framework::GraphQL | Framework::Grpc | Framework::OpenAPI | Framework::Protobuf) => make_pf(
            k,
            match k {
                Framework::GraphQL => "GraphQL",
                Framework::Grpc => "gRPC",
                Framework::OpenAPI => "OpenAPI",
                Framework::Protobuf => "Protocol Buffers",
                _ => "API contract",
            },
            API_SPEC_C,
            API_SPEC_M,
            None,
        ),
    }
}

// ——— Databases ———
static PG_C: &[&str] = &[
    "Migrations in VCS; reversible when possible",
    "Indexes for FK and filter columns — verify plans",
    "Avoid SELECT * in hot paths",
    "Use transactions for multi-row invariants",
    "Connection pool sizing and timeouts tuned per workload",
];
static MONGO_C: &[&str] = &[
    "Schema validation rules for critical collections",
    "Indexes aligned to query shapes",
    "Shard key chosen with write/read balance",
];
static REDIS_C: &[&str] = &[
    "Key TTLs for cache entries",
    "No secrets in plaintext values",
    "Eviction policy matches SLAs",
];
static ES_C: &[&str] = &[
    "Mappings explicit before production traffic",
    "Reindex strategy for schema changes",
];
static CH_C: &[&str] = &[
    "MergeTree tuning and partitioning by time when applicable",
];
static VEC_C: &[&str] = &[
    "Embedding model version pinned",
    "Metadata filters for privacy and freshness",
];

fn db_abs_label(a: DatabaseAbstraction) -> &'static str {
    match a {
        DatabaseAbstraction::Sqlx => "SQLx",
        DatabaseAbstraction::DieselOrm => "Diesel",
        DatabaseAbstraction::SeaOrm => "SeaORM",
        DatabaseAbstraction::PrismaOrm => "Prisma",
        DatabaseAbstraction::TypeOrm => "TypeORM",
        DatabaseAbstraction::Mongoose => "Mongoose",
        DatabaseAbstraction::SqlAlchemy => "SQLAlchemy",
        DatabaseAbstraction::Gorm => "GORM",
        DatabaseAbstraction::Hibernate => "Hibernate",
        DatabaseAbstraction::Ecto => "Ecto",
        DatabaseAbstraction::RedisCrate => "Redis (crate)",
        DatabaseAbstraction::GoMongoDriver => "mongo-driver",
    }
}

fn db_profile(d: Database) -> (&'static str, &'static [&'static str]) {
    match d {
        Database::PostgreSQL => ("PostgreSQL", PG_C),
        Database::MongoDB => ("MongoDB", MONGO_C),
        Database::Redis => ("Redis", REDIS_C),
        Database::Elasticsearch => ("Elasticsearch", ES_C),
        Database::ClickHouse => ("ClickHouse", CH_C),
        Database::Cassandra => ("Cassandra", PG_C),
        Database::TimescaleDB => ("TimescaleDB", PG_C),
        Database::VectorDb => ("Vector database", VEC_C),
        Database::DuckDB => ("DuckDB", PG_C),
        Database::BigQuery => ("BigQuery", CH_C),
    }
}

/// Heuristic DB + abstraction detection from common manifests.
pub fn detect_databases(root: &Path) -> (Vec<Database>, Vec<DatabaseAbstraction>) {
    let mut blob = String::new();
    for s in [
        cargo_txt(root),
        package_json_txt(root),
        pyproject_txt(root),
        requirements_txt(root),
        read_file_cap(&root.join("docker-compose.yml"), READ_CAP),
        read_file_cap(&root.join("docker-compose.yaml"), READ_CAP),
        gradle_and_maven_txt(root),
    ]
    .into_iter()
    .flatten()
    {
        blob.push_str(&s);
        blob.push('\n');
    }
    let b = blob.to_lowercase();
    let mut dbs: Vec<Database> = Vec::new();
    if b.contains("postgres") || b.contains("postgresql") {
        push_unique(&mut dbs, Database::PostgreSQL);
    }
    if b.contains("timescale") {
        push_unique(&mut dbs, Database::TimescaleDB);
    }
    if b.contains("mongo") {
        push_unique(&mut dbs, Database::MongoDB);
    }
    if b.contains("redis") {
        push_unique(&mut dbs, Database::Redis);
    }
    if b.contains("elasticsearch") || b.contains("opensearch") {
        push_unique(&mut dbs, Database::Elasticsearch);
    }
    if b.contains("clickhouse") {
        push_unique(&mut dbs, Database::ClickHouse);
    }
    if b.contains("cassandra") {
        push_unique(&mut dbs, Database::Cassandra);
    }
    if b.contains("pinecone")
        || b.contains("weaviate")
        || b.contains("milvus")
        || b.contains("qdrant")
        || b.contains("chroma")
        || b.contains("pgvector")
    {
        push_unique(&mut dbs, Database::VectorDb);
    }
    if b.contains("duckdb") {
        push_unique(&mut dbs, Database::DuckDB);
    }
    if b.contains("bigquery") {
        push_unique(&mut dbs, Database::BigQuery);
    }

    let mut abs: Vec<DatabaseAbstraction> = Vec::new();
    if let Some(c) = cargo_txt(root) {
        if c.contains("sqlx") {
            push_unique(&mut abs, DatabaseAbstraction::Sqlx);
        }
        if c.contains("diesel") {
            push_unique(&mut abs, DatabaseAbstraction::DieselOrm);
        }
        if c.contains("sea-orm") || c.contains("sea_orm") {
            push_unique(&mut abs, DatabaseAbstraction::SeaOrm);
        }
    }
    if let Some(p) = package_json_txt(root) {
        if p.contains("@prisma/client") {
            push_unique(&mut abs, DatabaseAbstraction::PrismaOrm);
        }
        if p.contains("typeorm") {
            push_unique(&mut abs, DatabaseAbstraction::TypeOrm);
        }
        if p.contains("mongoose") {
            push_unique(&mut abs, DatabaseAbstraction::Mongoose);
        }
    }
    if let Some(p) = pyproject_txt(root).or_else(|| requirements_txt(root))
        && p.contains("sqlalchemy")
    {
        push_unique(&mut abs, DatabaseAbstraction::SqlAlchemy);
    }
    if let Some(g) = read_file_cap(&root.join("go.mod"), READ_CAP) {
        if g.contains("gorm.io") {
            push_unique(&mut abs, DatabaseAbstraction::Gorm);
        }
        if g.contains("go.mongodb.org/mongo-driver") {
            push_unique(&mut abs, DatabaseAbstraction::GoMongoDriver);
        }
    }
    if let Some(t) = gradle_and_maven_txt(root) && t.contains("hibernate") {
        push_unique(&mut abs, DatabaseAbstraction::Hibernate);
    }
    if let Some(m) = read_file_cap(&root.join("mix.exs"), READ_CAP) && m.contains("ecto") {
        push_unique(&mut abs, DatabaseAbstraction::Ecto);
    }
    if let Some(c) = cargo_txt(root) && c.contains("redis") {
        push_unique(&mut abs, DatabaseAbstraction::RedisCrate);
    }

    (dbs, abs)
}

// ——— Data tools ———
static DT_KAFKA: &[&str] = &[
    "Idempotent consumers; explicit partitions",
    "Schema registry for Avro/Protobuf compatibility",
];
static DT_AIR: &[&str] = &[
    "DAGs pure — side effects in operators only",
    "SLAs and data quality checks as first-class tasks",
];
static DT_DBT: &[&str] = &[
    "Models documented; tests on critical metrics",
    "Environments separated (dev/stage/prod)",
];
static DT_SPARK: &[&str] = &[
    "Shuffle-heavy joins sized for cluster",
    "Checkpoint long streams",
];
static DT_SCRAPY: &[&str] = &[
    "robots.txt and rate limits respected",
];
static DT_GE: &[&str] = &[
    "Expectations versioned with datasets",
];

fn data_profile(t: DataTool) -> (&'static str, &'static [&'static str]) {
    match t {
        DataTool::Kafka => ("Apache Kafka", DT_KAFKA),
        DataTool::Airflow => ("Apache Airflow", DT_AIR),
        DataTool::Dbt => ("dbt", DT_DBT),
        DataTool::PandasEtl => ("Pandas (ETL)", DT_DBT),
        DataTool::PolarsEtl => ("Polars (ETL)", DT_DBT),
        DataTool::Spark => ("Apache Spark", DT_SPARK),
        DataTool::Scrapy => ("Scrapy", DT_SCRAPY),
        DataTool::GreatExpectations => ("Great Expectations", DT_GE),
    }
}

/// Detects batch/stream tooling from manifests.
pub fn detect_data_tools(root: &Path) -> Vec<DataTool> {
    let mut blob = String::new();
    for s in [
        pyproject_txt(root),
        requirements_txt(root),
        package_json_txt(root),
        read_file_cap(&root.join("docker-compose.yml"), READ_CAP),
    ]
    .into_iter()
    .flatten()
    {
        blob.push_str(&s);
        blob.push('\n');
    }
    let b = blob.to_lowercase();
    let mut v = Vec::new();
    if b.contains("kafka") {
        push_unique(&mut v, DataTool::Kafka);
    }
    if b.contains("airflow") {
        push_unique(&mut v, DataTool::Airflow);
    }
    if b.contains("dbt") {
        push_unique(&mut v, DataTool::Dbt);
    }
    if b.contains("pyspark") || b.contains("apache-spark") || b.contains("\"spark\"") {
        push_unique(&mut v, DataTool::Spark);
    }
    if b.contains("scrapy") {
        push_unique(&mut v, DataTool::Scrapy);
    }
    if b.contains("great_expectations") {
        push_unique(&mut v, DataTool::GreatExpectations);
    }
    if b.contains("pandas") {
        push_unique(&mut v, DataTool::PandasEtl);
    }
    if b.contains("polars") {
        push_unique(&mut v, DataTool::PolarsEtl);
    }
    v
}

// ——— Architecture ———
static ARC_MONO: &[&str] = &[
    "Modular boundaries inside one deployable",
    "Feature toggles instead of long-lived branches",
];
static ARC_MICRO: &[&str] = &[
    "Explicit service contracts; backwards compatible APIs",
    "Distributed tracing across calls",
    "Avoid chatty synchronous chains",
];
static ARC_DDD: &[&str] = &[
    "Ubiquitous language in code names",
    "Aggregates enforce invariants",
    "Domain services for cross-aggregate rules",
];
static ARC_CLEAN: &[&str] = &[
    "Dependencies point inward — frameworks at edges",
    "Use cases independent of delivery mechanism",
];
static ARC_CQRS: &[&str] = &[
    "Command/query models separated deliberately",
    "Idempotent event handlers",
];
static ARC_REST: &[&str] = &[
    "Resources noun-based; verbs via HTTP methods",
    "Consistent error envelope and pagination",
];

fn arch_profile(a: ArchitecturePattern) -> (&'static str, &'static [&'static str]) {
    match a {
        ArchitecturePattern::Monolith => ("Modular monolith", ARC_MONO),
        ArchitecturePattern::Microservices => ("Microservices", ARC_MICRO),
        ArchitecturePattern::Ddd => ("Domain-driven design", ARC_DDD),
        ArchitecturePattern::CleanArchitecture => ("Clean architecture", ARC_CLEAN),
        ArchitecturePattern::EventSourcingCqrs => ("Event sourcing / CQRS", ARC_CQRS),
        ArchitecturePattern::RestApiDesign => ("REST API design", ARC_REST),
    }
}

/// Very light structure hints (bounded filesystem peek).
pub fn detect_architecture_hints(root: &Path) -> Vec<ArchitecturePattern> {
    let mut out = Vec::new();
    let dc = root.join("docker-compose.yml");
    let dc2 = root.join("docker-compose.yaml");
    if dc.is_file() || dc2.is_file() {
        let txt = read_file_cap(&dc, READ_CAP)
            .or_else(|| read_file_cap(&dc2, READ_CAP))
            .unwrap_or_default();
        let images = txt.matches("image:").count();
        if images >= 3 {
            push_unique(&mut out, ArchitecturePattern::Microservices);
        }
    }
    if root.join("domain").is_dir() || root.join("domains").is_dir() {
        push_unique(&mut out, ArchitecturePattern::Ddd);
    }
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = 0usize;
    while let Some(d) = stack.pop() {
        if dirs > 120 {
            break;
        }
        dirs += 1;
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_lowercase();
            if name.contains("cqrs") || name.contains("event_sourcing") {
                push_unique(&mut out, ArchitecturePattern::EventSourcingCqrs);
            }
            if ent.path().is_dir() && !name.starts_with('.') && name != "target" && name != "node_modules"
            {
                stack.push(ent.path());
            }
        }
    }
    if root.join("openapi.yaml").is_file() || root.join("swagger.yaml").is_file() {
        push_unique(&mut out, ArchitecturePattern::RestApiDesign);
    }
    if out.is_empty() {
        push_unique(&mut out, ArchitecturePattern::Monolith);
    }
    out
}

// ——— Best-practice pools (2026) ———
static BP_SEC: &[&str] = &[
    "Treat secrets via environment or vault — never commit (.env in .gitignore)",
    "Validate all untrusted input at trust boundaries",
    "Least-privilege service accounts and IAM",
];
static BP_OBS: &[&str] = &[
    "Structured logs with correlation ids",
    "RED/USE metrics for services; traces on critical paths (OpenTelemetry)",
];
static BP_TEST: &[&str] = &[
    "Testing pyramid — fast unit, fewer integration, rare E2E",
    "Contract tests cover API compatibility",
];

fn select_best_practices(p: &ProjectProfile) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for s in BP_SEC.iter().take(2) {
        v.push(*s);
    }
    if !p.frameworks.is_empty() {
        for s in BP_OBS.iter().take(2) {
            v.push(*s);
        }
    }
    for s in BP_TEST.iter().take(1) {
        v.push(*s);
    }
    v
}

fn push_block(out: &mut String, title: &str, lines: impl Iterator<Item = impl AsRef<str>>) {
    out.push_str(title);
    out.push('\n');
    for line in lines {
        out.push_str(line.as_ref());
        out.push('\n');
    }
    out.push('\n');
}

fn take_n<'a>(slice: &'a [&'a str], n: usize) -> impl Iterator<Item = &'a str> + 'a {
    slice.iter().take(n).copied()
}

/// Caps UTF-8 `s` to `max` bytes without splitting codepoints badly.
fn cap_bytes(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("\n… (truncated to stay within injection budget)\n");
    s
}

/// Builds a formatted intelligence block (≤ `max` bytes). Priority: frameworks → DB → architecture → language → practices.
pub fn format_project_profile_capped(p: &ProjectProfile, max: usize) -> String {
    let mut s = String::from("=== Project intelligence ===\n\nStack: ");
    s.push_str(p.lang_profile.display_name);
    s.push_str(" · ");
    if p.frameworks.is_empty() {
        s.push_str("(no framework markers)");
    } else {
        let names: Vec<_> = p.framework_profiles.iter().map(|f| f.display_name).collect();
        s.push_str(&names.join(", "));
    }
    s.push_str(" · DB: ");
    if p.databases.is_empty() {
        s.push_str("none detected");
    } else {
        let dn: Vec<_> = p.databases.iter().map(|d| db_profile(*d).0).collect();
        s.push_str(&dn.join(", "));
    }
    if !p.db_abstractions.is_empty() {
        s.push_str(" · DB access: ");
        let labs: Vec<_> = p.db_abstractions.iter().map(|a| db_abs_label(*a)).collect();
        s.push_str(&labs.join(", "));
    }
    if !p.data_tools.is_empty() {
        s.push_str(" · Data tools: ");
        let labs: Vec<_> = p.data_tools.iter().map(|t| data_profile(*t).0).collect();
        s.push_str(&labs.join(", "));
    }
    s.push_str("\n\n");

    for fp in &p.framework_profiles {
        let head = format!("— {} —", fp.display_name);
        let c: Vec<_> = take_n(fp.conventions, 5).collect();
        let m: Vec<_> = take_n(fp.common_mistakes, 3).collect();
        push_block(&mut s, &head, c.into_iter().chain(std::iter::once("Avoid:")).chain(m));
    }

    for db in &p.databases {
        let (name, conv) = db_profile(*db);
        let head = format!("— Database: {name} —");
        push_block(&mut s, &head, take_n(conv, 5));
    }

    for tool in &p.data_tools {
        let (name, conv) = data_profile(*tool);
        let head = format!("— Data engineering: {name} —");
        push_block(&mut s, &head, take_n(conv, 5));
    }

    for a in &p.architecture_hints {
        let (name, conv) = arch_profile(*a);
        let head = format!("— Architecture hint: {name} —");
        push_block(&mut s, &head, take_n(conv, 5));
    }

    {
        let head = format!("— Language: {} —", p.lang_profile.display_name);
        let c: Vec<_> = take_n(p.lang_profile.conventions, 5).collect();
        push_block(&mut s, &head, c.into_iter());
    }

    s.push_str("Best practices for this stack:\n");
    for line in select_best_practices(p) {
        s.push_str(line);
        s.push('\n');
    }

    s.push_str("\n=== End project intelligence ===\n");
    cap_bytes(s, max)
}

/// Full structured [`ProjectProfile`] for `root` (never panics).
pub fn build_project_profile(root: &Path) -> ProjectProfile {
    let language = detect_language(root);
    let lang_profile = lang_profile(language);
    let frameworks = detect_frameworks(root, &language);
    let framework_profiles: Vec<FrameworkProfile> =
        frameworks.iter().copied().map(framework_profile).collect();
    let (databases, db_abstractions) = detect_databases(root);
    let data_tools = detect_data_tools(root);
    let architecture_hints = detect_architecture_hints(root);
    ProjectProfile {
        language,
        lang_profile,
        frameworks,
        framework_profiles,
        databases,
        db_abstractions,
        data_tools,
        architecture_hints,
    }
}

/// Convenience: [`build_project_profile`] + default 4000-byte cap.
pub fn format_project_intelligence_for_root(root: &Path) -> String {
    format_project_profile_capped(&build_project_profile(root), 4000)
}
