# Mihomo Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a concurrency-safe `ProxyManager` that starts, stops, restarts, and observes a Mihomo process.

**Architecture:** Keep one child process and one work directory behind `tokio::sync::Mutex`. Generate config before spawn, wait for the Mihomo controller port before reporting running, retain logs in the work directory, and clean up on stop or failed start.

## Task 1: Manager State

- [x] Add `ProxyManager` and private runtime state.
- [x] Test default stopped status.
- [x] Serialize start, stop, and restart operations.

## Task 2: Start And Failure Cleanup

- [x] Write config YAML and launch Mihomo with hidden Windows process flags.
- [x] Wait for controller readiness.
- [x] Mark failed starts and clean temporary files.
- [x] Test missing executable failure status.

## Task 3: Stop, Restart, Logs

- [x] Stop and wait for the child process.
- [x] Clean the work directory.
- [x] Add restart using stop then start.
- [x] Refresh status when the child exits.
- [x] Read bounded recent logs.

## Task 4: Verification

- [x] Run focused proxy tests.
- [x] Run `cargo fmt`, `cargo check --offline`, and `cargo test --offline`.
- [x] Mark phase 3 complete in `tasks.md`.
