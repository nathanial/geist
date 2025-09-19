# Repository Guidelines

## Project Structure & Module Organization
The viewer binary lives in `src/` with runtime glue in `src/app/` (events, init, step, watchers). Engine crates live in `crates/` for geometry, blocks, worldgen, meshing, lighting, runtime, structures, and IO; `crates/geist-render-raylib` is the sole GPU boundary. Shared assets sit in `assets/`, handcrafted content in `worlds/` and `schematics/`, and maintenance tools in `scripts/`. Keep generated captures in `showcase_output/` but out of version control.

## Build, Test, and Development Commands
- `cargo build --workspace` — builds every crate together.
- `cargo run -- run` — launches the fly-camera viewer; add flags like `--world flat` or `--seed <n>`.
- `cargo run -- schem report schematics/<file>.schem [--counts]` — audits schematic compatibility.
- `cargo test --workspace` — runs all unit tests; `src/stairs_tests.rs` only wires the harness.
- `cargo fmt` and `cargo clippy --workspace --all-targets` — required before review; treat warnings as failures.
- `scripts/flamegraph_wcc.sh` — optional flamegraph capture after `cargo build --profile flamegraph`.

## Coding Style & Naming Conventions
Use Rust 2024 defaults and rely on `cargo fmt` for layout. Modules, files, and functions stay snake_case; exported types and enums use UpperCamelCase with clear prefixes (`ChunkMeshCPU`, `LightingStore`). Break complex flows into helpers within the owning crate. If you must suppress a lint, pair the attribute with a brief rationale.

## Testing Guidelines
Place new unit tests beside the implementation inside `#[cfg(test)]` modules, using names like `generates_chunk_faces`. Integration or IO checks belong in crate-level `tests/` directories and should avoid GPU requirements. Run `cargo test --workspace` before every push and add regression coverage for bug fixes so the root harness can remain minimal.

## Commit & Pull Request Guidelines
Emulate the concise commit style in `git log`: a single imperative subject (e.g. `adjust lighting falloff`) and optional context lines. Group related crate work together and mention the crate in the subject when it clarifies scope. Pull requests should explain motivation, call out risky systems (meshing, streaming, lighting), link issues, and attach viewer screenshots or perf notes when behavior changes. Confirm fmt, clippy, build, and tests in the PR checklist.

## Assets & Configuration Tips
Add textures to `assets/blocks/` and update `assets/voxels/materials.toml` and `assets/voxels/blocks.toml` in the same change so hot reload stays consistent. Worldgen tweaks live in `assets/worldgen/worldgen.toml`; note any `--watch-worldgen` usage when sharing repro steps. When running tools outside the repo, set `GEIST_ASSETS` to the repository root so CLI commands find resources.
