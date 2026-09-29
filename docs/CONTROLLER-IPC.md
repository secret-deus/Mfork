# Native mihomo controller

The renderer sends API requests through `core_request` and receives logs,
traffic, memory and connections through Tauri channels. It does not use browser
`fetch` or browser WebSockets for the controller.

## Transport and lifecycle

- Linux/macOS: HTTP and WebSocket protocols over `external-controller-unix`.
  No TCP controller listener is generated. Each core session owns a fresh
  `/tmp/mfork-ctl-*/api.sock` inside a directory created atomically with mode
  `0700`. The directory is retained by in-flight requests/subscriptions and
  removed when its last owner is dropped.
- Windows/Android: Rust uses the existing authenticated loopback controller.
  These platforms also bypass browser networking; Android's notification
  service continues to receive its existing controller address and secret.
- mihomo bypasses secret authentication on Unix sockets and sets socket mode
  `0666`. Access is therefore protected by the parent directory, which remains
  private when a root service creates or recreates the socket. Do not move the
  socket directly into a shared writable directory.
- Subscription/override controller addresses are stripped, including TCP, TLS,
  Unix and named-pipe fields. The backend supplies the endpoint; the renderer
  can only send relative API paths and a session ID. Secrets stay in Rust.
- HTTP requests have bounded response sizes, timeouts and no proxy/redirect
  handling. Readiness requires valid `/version` and `/configs` responses.
- Each of the four streams reconnects with a 1–15 second backoff. Stop, exit
  and session replacement cancel subscriptions. Closing a frontend subscription
  while registration is pending also cancels a late registration.
- A UI initialization error reconciles the running core instead of dropping its
  identity. Service-managed cores are checked through the helper, since they
  have no GUI-owned `Child`. An unknown helper status retains the known session;
  it is not treated as a confirmed exit. Restart/apply remains available in the
  error state.

The privileged helper's authenticated IPC protocol is unchanged. No service
reinstallation is needed specifically for this transport change.

## Source verification — 2026-09-29

| Source | Latest branch / commit | Commit time (Asia/Shanghai) |
| --- | --- | --- |
| git.xhang.top/admin/Mfork | master / `4f0359ce4ea744e09c2f1c56673fe25168edcc1e` | 2026-09-07 17:30 |
| git.any.me.uk/admin/Mfork | feat/process-name-routing / `1ea96946ac72cf9c719df2751f844b508bc93af2` | 2026-09-28 23:06 |
| github.com/secret-deus/Mfork | codex/ubuntu-26.04-deb-0.1.20 / `3e805ce7e09796ac6ee549651b9bff9db8adda62` | 2026-09-29 00:20 |

The two private Gitea branch lists were checked through their authenticated UI
(respectively one and three branches, no tags). GitHub's API and SSH refs agree
on the newer 0.1.20 snapshot. The fetched Git tree is
`08c110ec565cac60ff6f8b857cd8be3db98a2119`, matching GitHub's Git-object API.
GitHub has a separate root/snapshot history, so the implementation branch starts
there and leaves the previous local `master` history intact. Gitea SSH was not
reachable during this check; no claim is made that the three repositories are
identical mirrors.

## Verification

```sh
pnpm test
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo test --manifest-path service/Cargo.toml --locked
MFORK_TEST_CORE=/absolute/path/to/mihomo cargo test \
  --manifest-path src-tauri/Cargo.toml --locked --lib \
  real_mihomo_unix_controller_smoke -- --ignored --nocapture
```

Frontend tests require a Node build supporting TypeScript stripping (official
Node 22.6+ / 24). The smoke test launches only an isolated temporary core, with
TUN and DNS disabled, no proxy listener, no subscription and no service install.
It checks readiness, proxy queries, mode changes, config reload and all four
WebSocket handshakes, including data on traffic/memory/connections. Mock tests
cover log/event forwarding, reconnection, cancellation, error reporting, socket
directory permissions and conflicting controller fields.

Enabling a real TUN or changing the user's active proxy is a separate manual
acceptance check. It is not performed by these tests. Linux is the locally
verified platform; macOS/Windows/Android still need platform build/runtime checks.

Local Ubuntu 26.04 verification on 2026-09-29: 55 frontend tests, 107 Rust app
tests, 27 service tests, the separate real-mihomo smoke test, TypeScript checking and production
frontend build passed. The smoke test used the installed mihomo v1.19.31 binary
with a separate temporary configuration. The running GUI/helper/core were not
restarted, and the installed application was not replaced.
