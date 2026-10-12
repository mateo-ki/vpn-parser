# Kernel Management Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add kernel discovery and version probing for Mihomo, Xray, and Sing-box while preserving existing Mihomo speed-test behavior.

**Architecture:** Create `src/kernel/mod.rs` as the single owner of kernel kinds, executable candidates, path discovery, and version parsing. Reuse its Mihomo discovery from `src/proxy_probe.rs`; keep runtime process management for the next phase.

## Task 1: Kernel Types

**Files:** Create `src/kernel/mod.rs`; modify `src/main.rs`.

- [x] Add `KernelKind`, `KernelInfo`, and `KernelManager` skeletons.
- [x] Add tests for stable display names and app-relative Mihomo candidates.
- [x] Register `mod kernel;` in `src/main.rs`.
- [x] Run `cargo test kernel --offline` and confirm the new tests pass.

## Task 2: Mihomo Discovery

**Files:** Modify `src/kernel/mod.rs`; modify `src/proxy_probe.rs`.

- [x] Add `find_first_existing_candidate` and `find_mihomo_executable`.
- [x] Support `MIHOMO_PATH`, executable-relative paths, and existing v2rayN paths.
- [x] Remove the duplicate local lookup from `src/proxy_probe.rs`.
- [x] Add a temporary-file discovery test.
- [x] Run focused proxy probe and kernel tests.

## Task 3: Version Detection

**Files:** Modify `src/kernel/mod.rs`.

- [x] Add `parse_version_output` and `read_kernel_version`.
- [x] Add `KernelManager::info_for_missing` and async `detect_mihomo`.
- [x] Add tests for first non-empty version output and missing kernel status.
- [x] Run the focused kernel tests.

## Task 4: Verification

**Files:** Modify `tasks.md`.

- [x] Run `cargo fmt`.
- [x] Run `cargo check --offline`.
- [x] Run `cargo test --offline`.
- [x] Mark phase 1 items complete in `tasks.md`.
- [x] Inspect `git status --short`.
