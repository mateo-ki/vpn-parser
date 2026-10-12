# Windows System Proxy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Safely enable the Windows system proxy while Mihomo runs and restore the prior settings on stop or failure.

**Architecture:** Put registry operations behind a synchronous `SystemProxyControl` trait. The Windows implementation uses `reg.exe` and WinINet refresh notifications; tests use pure parsing and a fake controller. `ProxyManager` stores the captured snapshot for restoration.

## Tasks

- [x] Add snapshot and controller types in `src/system_proxy/mod.rs`.
- [x] Add registry output parsing tests.
- [x] Implement Windows capture, enable, restore, and WinINet notification.
- [x] Inject the controller into `ProxyManager`.
- [x] Restore settings on stop, startup failure, abnormal exit, and normal app exit.
- [x] Run focused and full tests.
