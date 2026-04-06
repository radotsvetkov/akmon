//! Language, framework, database, and architecture intelligence for project context injection.
//!
//! Detection reads only manifest and marker files under the project root (bounded walks).
//! Many public enums mirror external ecosystem names; per-variant docs would be noisy.

#![allow(missing_docs)]

use std::fs;
use std::io::Read as _;
use std::path::Path;

// ——— limits ———

const READ_CAP: usize = 256 * 1024;
const WALK_MAX_DIRS: usize = 400;
const SWIFT_SCAN_MAX: usize = 40;

// ——— Section 1: Languages ———

/// Primary implementation language inferred from repository markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// Rust (`Cargo.toml`).
    Rust,
    /// Python (`pyproject.toml`, `requirements.txt`, `setup.py`).
    Python,
    /// TypeScript (`tsconfig.json`).
    TypeScript,
    /// JavaScript (`package.json` without `tsconfig.json`).
    JavaScript,
    /// Go (`go.mod`).
    Go,
    /// Java (`pom.xml`, `build.gradle`, or JVM layout without Kotlin-first signals).
    Java,
    /// C# (`*.csproj`, `*.sln`).
    CSharp,
    /// Elixir (`mix.exs`).
    Elixir,
    /// Ruby (`Gemfile`).
    Ruby,
    /// Swift (`Package.swift`, `*.xcodeproj`).
    Swift,
    /// Kotlin (Gradle with Kotlin plugin / `build.gradle.kts`).
    Kotlin,
    /// Dart / Flutter (`pubspec.yaml`).
    Dart,
    /// C++ (`CMakeLists.txt`, or `.cpp`/`.cc` sources under root tree).
    Cpp,
    /// Zig (`build.zig`).
    Zig,
    /// No known marker matched.
    Unknown,
}

/// Static guidance for one [`Language`].
#[derive(Debug, Clone, Copy)]
pub struct LangProfile {
    /// Matching [`Language`].
    pub language: Language,
    /// Human-readable name for prompts.
    pub display_name: &'static str,
    /// Conventions the agent should follow.
    pub conventions: &'static [&'static str],
    /// Common pitfalls.
    pub common_mistakes: &'static [&'static str],
    /// How to test.
    pub testing_approach: &'static str,
    /// Error handling style.
    pub error_handling: &'static str,
    /// Typical tools.
    pub toolchain: &'static str,
}

// ——— Section 2: Frameworks ———

/// Detected framework or major library stack element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Framework {
    // Rust web
    Axum,
    Actix,
    Rocket,
    Warp,
    // Rust CLI/TUI
    Clap,
    Ratatui,
    // Rust async/systems
    Tokio,
    AsyncStd,
    Rayon,
    // Rust ORM/data
    Sqlx,
    Diesel,
    SeaOrm,
    // Rust other
    Tauri,
    Bevy,
    Leptos,
    // Python web
    FastAPI,
    Django,
    Flask,
    Starlette,
    Litestar,
    // Python data/ML
    Pandas,
    Polars,
    NumPy,
    Pydantic,
    PyTorch,
    TensorFlow,
    HuggingFace,
    LangChain,
    LlamaIndex,
    // Python async jobs
    Celery,
    // TS/JS front-end
    NextJs,
    React,
    Vue,
    Svelte,
    SolidJs,
    Angular,
    Astro,
    Remix,
    // TS/JS backend
    NestJs,
    Express,
    Fastify,
    Hono,
    ElysiaJs,
    // TS/JS data layer
    Prisma,
    Drizzle,
    Trpc,
    Tanstack,
    // Go web
    Gin,
    Echo,
    Chi,
    Fiber,
    Gorilla,
    Cobra,
    // Java
    SpringBoot,
    Quarkus,
    Micronaut,
    VertX,
    // C#
    AspNetCore,
    MauiBlazor,
    // Elixir
    Phoenix,
    LiveView,
    // Mobile — iOS
    SwiftUI,
    UIKit,
    Combine,
    // Mobile — Android
    JetpackCompose,
    AndroidView,
    // Mobile — cross
    FlutterFramework,
    ReactNative,
    ExpoFramework,
    KotlinMultiplatform,
    // Cross-cutting APIs
    GraphQL,
    Grpc,
    OpenAPI,
    Protobuf,
}

/// Guidance for one [`Framework`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameworkProfile {
    /// Matching framework.
    pub framework: Framework,
    /// Short label for prompts.
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
    pub common_mistakes: &'static [&'static str],
    /// Optional architecture blurb (one line).
    pub patterns: Option<&'static str>,
}

// ——— Section 3: Databases ———

/// Persistent data store or search engine detected from config/deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Database {
    PostgreSQL,
    MongoDB,
    Redis,
    Elasticsearch,
    ClickHouse,
    Cassandra,
    TimescaleDB,
    VectorDb,
    DuckDB,
    BigQuery,
}

/// ORM or DB client layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DatabaseAbstraction {
    Sqlx,
    DieselOrm,
    SeaOrm,
    PrismaOrm,
    TypeOrm,
    Mongoose,
    SqlAlchemy,
    Gorm,
    Hibernate,
    Ecto,
    RedisCrate,
    GoMongoDriver,
}

/// DB-oriented profile (conventions only).
#[derive(Debug, Clone, Copy)]
pub struct DatabaseProfile {
    pub database: Database,
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
}

// ——— Section 4: Data engineering ———

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataTool {
    Kafka,
    Airflow,
    Dbt,
    PandasEtl,
    PolarsEtl,
    Spark,
    Scrapy,
    GreatExpectations,
}

#[derive(Debug, Clone, Copy)]
pub struct DataToolProfile {
    pub tool: DataTool,
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
}

// ——— Section 5: Architecture ———

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchitecturePattern {
    Monolith,
    Microservices,
    Ddd,
    CleanArchitecture,
    EventSourcingCqrs,
    RestApiDesign,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchitectureProfile {
    pub pattern: ArchitecturePattern,
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
}

// ——— Section 6: Best practices (pools) ———

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BestPracticeArea {
    CodeQuality,
    Documentation,
    Testing,
    Security,
    Observability,
    Performance,
}

// ——— Section 7: Schema / modeling ———

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemaPractice {
    RelationalSchemaDesign,
    DataModelingPythonic,
}

#[derive(Debug, Clone, Copy)]
pub struct SchemaProfile {
    pub practice: SchemaPractice,
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
}

// ——— Section 8: Web / frontend ———

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebFrontend {
    CssTailwind,
    Html5Semantic,
}

#[derive(Debug, Clone, Copy)]
pub struct WebProfile {
    pub area: WebFrontend,
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
}

// ——— Section 9: Design patterns ———

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DesignPattern {
    Repository,
    CircuitBreaker,
    Outbox,
    Pipeline,
    Saga,
}

#[derive(Debug, Clone, Copy)]
pub struct DesignPatternProfile {
    pub pattern: DesignPattern,
    pub display_name: &'static str,
    pub conventions: &'static [&'static str],
}

// ——— Aggregated profile ———

/// Everything [`build_project_profile`] infers for the repository root.
#[derive(Debug, Clone)]
pub struct ProjectProfile {
    /// Inferred [`Language`].
    pub language: Language,
    /// Static [`LangProfile`] for [`Self::language`].
    pub lang_profile: &'static LangProfile,
    /// Frameworks detected from manifests and bounded file scans.
    pub frameworks: Vec<Framework>,
    /// Resolved guidance for each entry in [`Self::frameworks`].
    pub framework_profiles: Vec<FrameworkProfile>,
    pub databases: Vec<Database>,
    pub db_abstractions: Vec<DatabaseAbstraction>,
    pub data_tools: Vec<DataTool>,
    pub architecture_hints: Vec<ArchitecturePattern>,
}

/// Infers visible language markers under `root` (first match wins by priority).
pub fn detect_language(root: &Path) -> Language {
    if root.join("Cargo.toml").is_file() {
        return Language::Rust;
    }
    if root.join("pyproject.toml").is_file()
        || root.join("requirements.txt").is_file()
        || root.join("setup.py").is_file()
    {
        return Language::Python;
    }
    if root.join("tsconfig.json").is_file() {
        return Language::TypeScript;
    }
    if root.join("package.json").is_file() {
        return Language::JavaScript;
    }
    if root.join("go.mod").is_file() {
        return Language::Go;
    }
    if detect_kotlin_gradle(root) {
        return Language::Kotlin;
    }
    if root.join("pom.xml").is_file()
        || root.join("build.gradle").is_file()
        || root.join("build.gradle.kts").is_file()
    {
        return Language::Java;
    }
    if has_glob_csproj_or_sln(root) {
        return Language::CSharp;
    }
    if root.join("mix.exs").is_file() {
        return Language::Elixir;
    }
    if root.join("Gemfile").is_file() {
        return Language::Ruby;
    }
    if root.join("Package.swift").is_file() || has_xcodeproj(root) {
        return Language::Swift;
    }
    if root.join("pubspec.yaml").is_file() {
        return Language::Dart;
    }
    if root.join("CMakeLists.txt").is_file()
        || count_ext_under_root(root, &["cpp", "cc", "cxx"], 3) > 0
    {
        return Language::Cpp;
    }
    if root.join("build.zig").is_file() {
        return Language::Zig;
    }
    Language::Unknown
}

fn detect_kotlin_gradle(root: &Path) -> bool {
    for name in ["build.gradle.kts", "build.gradle"] {
        let p = root.join(name);
        if !p.is_file() {
            continue;
        }
        if let Some(t) = read_file_cap(&p, READ_CAP)
            && (t.contains("kotlin(")
                || t.contains("org.jetbrains.kotlin")
                || t.contains("kotlin-android"))
        {
            return true;
        }
    }
    false
}

fn has_xcodeproj(root: &Path) -> bool {
    let Ok(rd) = fs::read_dir(root) else {
        return false;
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.ends_with(".xcodeproj") {
            return true;
        }
    }
    false
}

fn has_glob_csproj_or_sln(root: &Path) -> bool {
    let Ok(rd) = fs::read_dir(root) else {
        return false;
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        if lower.ends_with(".csproj") || lower.ends_with(".sln") {
            return true;
        }
    }
    false
}

fn read_file_cap(path: &Path, max: usize) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}

fn count_ext_under_root(root: &Path, exts: &[&str], cap: usize) -> usize {
    let mut n = 0usize;
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = 0usize;
    while let Some(d) = stack.pop() {
        if dirs >= WALK_MAX_DIRS {
            break;
        }
        dirs += 1;
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            if n >= cap {
                return n;
            }
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    "target" | "node_modules" | "vendor" | "build" | ".git"
                ) {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.contains(&e))
            {
                n += 1;
            }
        }
    }
    n
}

fn cargo_txt(root: &Path) -> Option<String> {
    read_file_cap(&root.join("Cargo.toml"), READ_CAP)
}

fn package_json_txt(root: &Path) -> Option<String> {
    read_file_cap(&root.join("package.json"), READ_CAP)
}

fn requirements_txt(root: &Path) -> Option<String> {
    let p = root.join("requirements.txt");
    if p.is_file() {
        read_file_cap(&p, READ_CAP)
    } else {
        None
    }
}

fn pyproject_txt(root: &Path) -> Option<String> {
    read_file_cap(&root.join("pyproject.toml"), READ_CAP)
}

fn gradle_and_maven_txt(root: &Path) -> Option<String> {
    for n in ["build.gradle.kts", "build.gradle", "pom.xml"] {
        let p = root.join(n);
        if p.is_file() {
            return read_file_cap(&p, READ_CAP);
        }
    }
    None
}

fn pubspec_txt(root: &Path) -> Option<String> {
    read_file_cap(&root.join("pubspec.yaml"), READ_CAP)
}

fn swift_sources_import(root: &Path, needle: &str) -> bool {
    let mut stack = vec![root.to_path_buf()];
    let mut scanned = 0usize;
    let mut dirs = 0usize;
    while let Some(d) = stack.pop() {
        if dirs >= WALK_MAX_DIRS || scanned >= SWIFT_SCAN_MAX {
            break;
        }
        dirs += 1;
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            if scanned >= SWIFT_SCAN_MAX {
                return false;
            }
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    "target" | "node_modules" | "vendor" | "build" | ".git"
                ) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "swift") {
                scanned += 1;
                if let Some(t) = read_file_cap(&path, 16 * 1024)
                    && t.contains(needle)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn has_proto_or_graphql_openapi(root: &Path, frameworks: &mut Vec<Framework>) {
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = 0usize;
    while let Some(d) = stack.pop() {
        if dirs >= WALK_MAX_DIRS {
            break;
        }
        dirs += 1;
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if matches!(name.as_str(), "target" | "node_modules" | "vendor" | ".git") {
                    continue;
                }
                stack.push(path);
            } else {
                let lower = name.to_lowercase();
                if lower.ends_with(".proto") {
                    push_unique(frameworks, Framework::Grpc);
                    push_unique(frameworks, Framework::Protobuf);
                } else if lower.ends_with(".graphql") || lower.ends_with(".gql") {
                    push_unique(frameworks, Framework::GraphQL);
                } else if lower == "openapi.yaml"
                    || lower == "openapi.yml"
                    || lower == "swagger.yaml"
                    || lower == "swagger.yml"
                {
                    push_unique(frameworks, Framework::OpenAPI);
                }
            }
        }
    }
}

fn push_unique<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

fn dep_hit(hay: &str, needle: &str) -> bool {
    let q = format!("\"{needle}\"");
    let q2 = format!("'{needle}'");
    hay.contains(&q) || hay.contains(&q2)
}

fn first_csproj_contents(root: &Path) -> Option<String> {
    let dbp = root.join("Directory.Build.props");
    if dbp.is_file()
        && let Some(t) = read_file_cap(&dbp, READ_CAP)
    {
        return Some(t);
    }
    let Ok(rd) = fs::read_dir(root) else {
        return None;
    };
    for ent in rd.flatten() {
        let n = ent.file_name().to_string_lossy().into_owned();
        if n.to_lowercase().ends_with(".csproj") {
            return read_file_cap(&ent.path(), READ_CAP);
        }
    }
    None
}

/// Frameworks implied by manifests plus cross-cutting API markers.
pub fn detect_frameworks(root: &Path, language: &Language) -> Vec<Framework> {
    let mut out = Vec::new();

    match language {
        Language::Rust => {
            if let Some(c) = cargo_txt(root) {
                let pairs: &[(&str, Framework)] = &[
                    ("axum", Framework::Axum),
                    ("actix-web", Framework::Actix),
                    ("actix_web", Framework::Actix),
                    ("rocket", Framework::Rocket),
                    ("warp", Framework::Warp),
                    ("clap", Framework::Clap),
                    ("ratatui", Framework::Ratatui),
                    ("tokio", Framework::Tokio),
                    ("async-std", Framework::AsyncStd),
                    ("async_std", Framework::AsyncStd),
                    ("rayon", Framework::Rayon),
                    ("sqlx", Framework::Sqlx),
                    ("diesel", Framework::Diesel),
                    ("sea-orm", Framework::SeaOrm),
                    ("sea_orm", Framework::SeaOrm),
                    ("tauri", Framework::Tauri),
                    ("bevy", Framework::Bevy),
                    ("leptos", Framework::Leptos),
                ];
                for (n, fw) in pairs {
                    if c.contains(n) {
                        push_unique(&mut out, *fw);
                    }
                }
            }
        }
        Language::Python => {
            let blobs = [pyproject_txt(root), requirements_txt(root)];
            for b in blobs.into_iter().flatten() {
                let pairs: &[(&str, Framework)] = &[
                    ("fastapi", Framework::FastAPI),
                    ("django", Framework::Django),
                    ("flask", Framework::Flask),
                    ("starlette", Framework::Starlette),
                    ("litestar", Framework::Litestar),
                    ("pandas", Framework::Pandas),
                    ("polars", Framework::Polars),
                    ("numpy", Framework::NumPy),
                    ("pydantic", Framework::Pydantic),
                    ("torch", Framework::PyTorch),
                    ("tensorflow", Framework::TensorFlow),
                    ("transformers", Framework::HuggingFace),
                    ("langchain", Framework::LangChain),
                    ("llama-index", Framework::LlamaIndex),
                    ("llama_index", Framework::LlamaIndex),
                    ("celery", Framework::Celery),
                ];
                for (n, fw) in pairs {
                    if b.contains(n) {
                        push_unique(&mut out, *fw);
                    }
                }
            }
        }
        Language::TypeScript | Language::JavaScript => {
            if let Some(pj) = package_json_txt(root) {
                let mut check = |name: &str, fw: Framework| {
                    if dep_hit(&pj, name) {
                        push_unique(&mut out, fw);
                    }
                };
                check("next", Framework::NextJs);
                check("react", Framework::React);
                check("vue", Framework::Vue);
                check("svelte", Framework::Svelte);
                check("solid-js", Framework::SolidJs);
                check("@angular/core", Framework::Angular);
                check("astro", Framework::Astro);
                check("@remix-run/react", Framework::Remix);
                check("@nestjs/core", Framework::NestJs);
                check("express", Framework::Express);
                check("fastify", Framework::Fastify);
                check("hono", Framework::Hono);
                check("elysia", Framework::ElysiaJs);
                check("@prisma/client", Framework::Prisma);
                check("drizzle-orm", Framework::Drizzle);
                check("@trpc/server", Framework::Trpc);
                check("@tanstack/react-query", Framework::Tanstack);
                check("react-native", Framework::ReactNative);
                check("expo", Framework::ExpoFramework);
                check("@expo/router", Framework::ExpoFramework);
                if dep_hit(&pj, "graphql") {
                    push_unique(&mut out, Framework::GraphQL);
                }
            }
        }
        Language::Go => {
            if let Some(g) = read_file_cap(&root.join("go.mod"), READ_CAP) {
                let pairs: &[(&str, Framework)] = &[
                    ("github.com/gin-gonic/gin", Framework::Gin),
                    ("github.com/labstack/echo", Framework::Echo),
                    ("github.com/go-chi/chi", Framework::Chi),
                    ("github.com/gofiber/fiber", Framework::Fiber),
                    ("github.com/gorilla/mux", Framework::Gorilla),
                    ("github.com/spf13/cobra", Framework::Cobra),
                ];
                for (n, fw) in pairs {
                    if g.contains(n) {
                        push_unique(&mut out, *fw);
                    }
                }
            }
        }
        Language::Java => {
            if let Some(t) = gradle_and_maven_txt(root) {
                if t.contains("spring-boot") || t.contains("springframework") {
                    push_unique(&mut out, Framework::SpringBoot);
                }
                if t.contains("quarkus") {
                    push_unique(&mut out, Framework::Quarkus);
                }
                if t.contains("micronaut") {
                    push_unique(&mut out, Framework::Micronaut);
                }
                if t.contains("vertx") || t.contains("io.vertx") {
                    push_unique(&mut out, Framework::VertX);
                }
            }
        }
        Language::CSharp => {
            if let Some(t) = first_csproj_contents(root) {
                if t.contains("Microsoft.NET.Sdk.Web") || t.contains("AspNetCore") {
                    push_unique(&mut out, Framework::AspNetCore);
                }
                if t.contains("Maui") || t.contains("Blazor") {
                    push_unique(&mut out, Framework::MauiBlazor);
                }
            }
        }
        Language::Elixir => {
            if let Some(m) = read_file_cap(&root.join("mix.exs"), READ_CAP) {
                if m.contains("phoenix") {
                    push_unique(&mut out, Framework::Phoenix);
                }
                if m.contains("phoenix_live_view") || m.contains("live_view") {
                    push_unique(&mut out, Framework::LiveView);
                }
            }
        }
        Language::Swift => {
            if let Some(m) = read_file_cap(&root.join("Package.swift"), READ_CAP) {
                let l = m.to_lowercase();
                if l.contains("swiftui") {
                    push_unique(&mut out, Framework::SwiftUI);
                }
                if l.contains("uikit") {
                    push_unique(&mut out, Framework::UIKit);
                }
                if l.contains("combine") {
                    push_unique(&mut out, Framework::Combine);
                }
            }
            if swift_sources_import(root, "import SwiftUI") {
                push_unique(&mut out, Framework::SwiftUI);
            }
            if swift_sources_import(root, "import UIKit") {
                push_unique(&mut out, Framework::UIKit);
            }
            if swift_sources_import(root, "import Combine") {
                push_unique(&mut out, Framework::Combine);
            }
        }
        Language::Kotlin => {
            if let Some(t) = gradle_and_maven_txt(root) {
                let l = t.to_lowercase();
                if l.contains("jetpack compose")
                    || l.contains("compose")
                    || l.contains("androidx.compose")
                {
                    push_unique(&mut out, Framework::JetpackCompose);
                } else {
                    push_unique(&mut out, Framework::AndroidView);
                }
                if l.contains("kotlin multiplatform") || l.contains("kotlin(\"multiplatform\")") {
                    push_unique(&mut out, Framework::KotlinMultiplatform);
                }
            }
        }
        Language::Dart => {
            if let Some(y) = pubspec_txt(root) {
                if y.contains("flutter:") || y.contains("flutter_sdk") {
                    push_unique(&mut out, Framework::FlutterFramework);
                }
                if y.contains("go_router") {
                    push_unique(&mut out, Framework::FlutterFramework);
                }
                if y.contains("bloc") || y.contains("flutter_bloc") {
                    push_unique(&mut out, Framework::FlutterFramework);
                }
            }
        }
        _ => {}
    }

    has_proto_or_graphql_openapi(root, &mut out);
    out
}

include!("lang_profile_data.inc");

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detect_language_markers() {
        let tmp = tempdir().expect("tempdir");
        let root = tmp.path();
        assert_eq!(detect_language(root), Language::Unknown);

        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .expect("write");
        assert_eq!(detect_language(root), Language::Rust);

        let py = tempdir().expect("tempdir");
        fs::write(py.path().join("pyproject.toml"), "[project]\nname=\"p\"\n").expect("write");
        assert_eq!(detect_language(py.path()), Language::Python);

        let ts = tempdir().expect("tempdir");
        fs::write(ts.path().join("tsconfig.json"), "{}").expect("write");
        assert_eq!(detect_language(ts.path()), Language::TypeScript);

        let go = tempdir().expect("tempdir");
        fs::write(go.path().join("go.mod"), "module x\n").expect("write");
        assert_eq!(detect_language(go.path()), Language::Go);

        let sw = tempdir().expect("tempdir");
        fs::write(sw.path().join("Package.swift"), "// swift\n").expect("write");
        assert_eq!(detect_language(sw.path()), Language::Swift);

        let kt = tempdir().expect("tempdir");
        fs::write(
            kt.path().join("build.gradle.kts"),
            "plugins { id(\"org.jetbrains.kotlin.android\") }\n",
        )
        .expect("write");
        assert_eq!(detect_language(kt.path()), Language::Kotlin);

        let dart = tempdir().expect("tempdir");
        fs::write(dart.path().join("pubspec.yaml"), "name: x\n").expect("write");
        assert_eq!(detect_language(dart.path()), Language::Dart);
    }

    #[test]
    fn detect_frameworks_rust_python_node() {
        let r = tempdir().expect("tempdir");
        fs::write(
            r.path().join("Cargo.toml"),
            "[dependencies]\naxum = \"0.7\"\nclap = \"4\"\nratatui = \"0.26\"\ntokio = { version = \"1\", features = [\"full\"] }\n",
        )
        .expect("write");
        let lang = detect_language(r.path());
        let fw = detect_frameworks(r.path(), &lang);
        assert!(fw.contains(&Framework::Axum));
        assert!(fw.contains(&Framework::Clap));
        assert!(fw.contains(&Framework::Ratatui));
        assert!(fw.contains(&Framework::Tokio));

        let py = tempdir().expect("tempdir");
        fs::write(py.path().join("requirements.txt"), "fastapi\nuvicorn\n").expect("write");
        let l = detect_language(py.path());
        let pf = detect_frameworks(py.path(), &l);
        assert!(pf.contains(&Framework::FastAPI));

        let node = tempdir().expect("tempdir");
        fs::write(
            node.path().join("package.json"),
            r#"{"dependencies":{"next":"14","react-native":"0.74","expo":"50"}}"#,
        )
        .expect("write");
        let lj = detect_language(node.path());
        assert_eq!(lj, Language::JavaScript);
        let nf = detect_frameworks(node.path(), &lj);
        assert!(nf.contains(&Framework::NextJs));
        assert!(nf.contains(&Framework::ReactNative));
        assert!(nf.contains(&Framework::ExpoFramework));
    }

    #[test]
    fn proto_and_graphql_files_add_frameworks() {
        let d = tempdir().expect("tempdir");
        fs::write(d.path().join("go.mod"), "module x\n").expect("write");
        fs::create_dir_all(d.path().join("api")).expect("dir");
        fs::write(d.path().join("api/x.proto"), "syntax = \"proto3\";\n").expect("write");
        fs::write(d.path().join("api/q.graphql"), "type Query { x: Int }\n").expect("write");
        let lang = detect_language(d.path());
        let fw = detect_frameworks(d.path(), &lang);
        assert!(fw.contains(&Framework::Grpc));
        assert!(fw.contains(&Framework::Protobuf));
        assert!(fw.contains(&Framework::GraphQL));
    }

    #[test]
    fn build_project_profile_never_panics_and_intel_under_cap() {
        let tmp = tempdir().expect("tempdir");
        let p = build_project_profile(tmp.path());
        assert_eq!(p.language, Language::Unknown);
        let s = format_project_intelligence_for_root(tmp.path());
        assert!(s.len() <= 4000);
        assert!(s.contains("=== Project intelligence ==="));

        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|x| x.parent())
            .expect("workspace");
        let p2 = build_project_profile(repo);
        assert_eq!(p2.language, Language::Rust);
        let big = format_project_profile_capped(&p2, 4000);
        assert!(big.len() <= 4000);
    }

    #[test]
    fn every_language_has_nonempty_lang_profile() {
        for lang in [
            Language::Rust,
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Java,
            Language::CSharp,
            Language::Elixir,
            Language::Ruby,
            Language::Swift,
            Language::Kotlin,
            Language::Dart,
            Language::Cpp,
            Language::Zig,
            Language::Unknown,
        ] {
            let p = lang_profile(lang);
            assert!(!p.display_name.is_empty());
            assert!(!p.conventions.is_empty());
        }
    }
}
