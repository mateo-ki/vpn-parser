# Proxy Runtime Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add proxy runtime state types, Mihomo configuration generation for one selected node, and managed temporary work directories.

**Architecture:** Create `src/proxy/mod.rs` as the boundary for proxy runtime concerns. Reuse `nodes_to_clash_proxies` for protocol conversion, produce a minimal Mihomo rule-mode configuration, and keep process lifecycle out of this phase.

## Task 1: Runtime State Types

- [x] Add `ProxyMode`, `ProxyState`, `ProxyRuntimeStatus`, and `MihomoConfigOptions`.
- [x] Add tests for stable default stopped status and default ports.
- [x] Register `mod proxy;` in `src/main.rs`.

## Task 2: Mihomo Configuration Generator

- [x] Add a failing test using a VLESS node.
- [x] Implement `build_mihomo_config` by reusing `nodes_to_clash_proxies`.
- [x] Reject unsupported nodes with a clear error.
- [x] Verify mixed port, controller, proxy group, and `MATCH,PROXY` rule.

## Task 3: Work Directory Management

- [x] Add `ProxyWorkDir::create` under the system temporary directory.
- [x] Expose config and log paths.
- [x] Add explicit async cleanup.
- [x] Add a test that creates and removes the directory.

## Task 4: Verification

- [x] Run `cargo fmt`.
- [x] Run `cargo check --offline`.
- [x] Run `cargo test --offline`.
- [x] Mark phase 2 complete in `tasks.md`.
