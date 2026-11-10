# Task: Agents get out on a Linux Docker engine

**From:** the sandbox suite in CI (task 0, 2026-11-09)
**Serves:** trust (the same isolation everywhere), and Nandi Cloud later (no Docker Desktop there)
**Size:** S

Found while making the isolation claims run in CI on a Linux runner. Both work on Docker Desktop (Windows, macOS) and break on a plain Linux engine.

## What's wrong

- **The proxy is out of reach.** The egress proxy binds `127.0.0.1` (`egress::bind_proxy_listener`). The relay reaches it as `host.docker.internal`, which `host-gateway` maps to the bridge gateway (`172.17.0.1`) on Linux. Nothing listens there, so every agent's traffic fails. The sandbox suite binds its own proxy on the bridge gateway to test the isolation; the app does not.
- **The container's uid is fixed.** Containers run as `1000:1000` (`runtime::CONTAINER_USER`, `--user=1000:1000`). A host user with another uid (GitHub's runner is 1001) owns the workdir the container can't write. The suite opens its folders up (`0777`) to work around it.

## What to do

- [ ] Listen where the relay reaches the proxy on Linux too (the bridge gateway's address, found from `docker network inspect bridge`), without exposing it on other interfaces. The token check stays the guard.
- [ ] Either run containers as the host user's uid on Linux, or make the workdir and shared folders writable for uid 1000 when they're created (and say which in `docs/developer/agent-runtime.md`).
- [ ] Then drop the suite's work-arounds (`proxy_host_ip`, `open_up` in `src-tauri/src/sandbox_suite.rs`) so CI tests the app's own setup.

## Done when

An agent on a Linux engine reaches an allowed site through the proxy, and the sandbox suite passes with no work-arounds.
