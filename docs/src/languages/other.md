# Other Languages

Akmon includes profiles beyond Rust, Python, TypeScript, and Go, for example **JavaScript**, **Java**, **C#**, **Elixir**, **Swift**, **Kotlin**, **Dart**, **C++**, **Zig**, and more. Detection uses manifests (`pom.xml`, `*.csproj`, `mix.exs`, `Package.swift`, `pubspec.yaml`, …).

## JavaScript (no `tsconfig.json`)

Conventions steer toward ES modules, `const`/`let`, modern syntax, and `async`/`await`.

## Java

Spring / Quarkus / Micronaut hints: records for DTOs, constructor injection, `Optional`, try-with-resources.

## C#

ASP.NET Core: nullable reference types, records, async all the way through.

## Elixir

Phoenix / LiveView: contexts, supervisors, `{:ok, _}` / `{:error, _}` tuples.

## Swift / iOS

SwiftUI patterns, `async`/`await`, avoiding force unwraps in production paths.

## Kotlin / Android

Compose-first guidance, coroutines, data classes.

## Dart / Flutter

`const` constructors, separation of UI and logic, common routing libraries.

Run **`akmon init`** so `AKMON.md` captures stack-specific conventions your team cares about beyond auto-detection.
