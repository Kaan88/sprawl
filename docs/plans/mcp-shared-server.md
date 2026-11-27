# One shared MCP server or one per session

Status, 2026-11-27: **keep one stdio server per Claude session.** Revisit only if memory from
repeated scans becomes a problem. Context: 1.0.11 fixed the MCP hangs inside the per-session
design (separate stdin/workers/stdout, deadlines, cancellation), so the hangs are no longer a
reason to change how sessions connect.

## Today

Each Claude session starts its own `sprawl.exe --mcp` over stdio. With Sprawl's window open, it
forwards calls over the window's named pipe (`pipe.rs`), which checks the caller's Windows account
and elevation. With the window closed, it answers from its own scans or History. An idle
process uses about 10 MB and almost no CPU.

## One shared server (local HTTP)

Pros
- One process instead of one per session; small gain on its own.
- Scans shared between sessions. A scan done without the window lives in that process's memory
  (hundreds of MB for a whole C: drive); two sessions scanning the same drive hold two copies.
- One copy of facts and findings per scan.

Cons
- Weaker security. The pipe sees the caller's account and elevation, which keeps an unelevated
  client to the scan on screen unless `adminMcpDisk` is on. HTTP can't, so it needs an access
  token: any program that has the token gets that access. A localhost port also needs
  protection against browser attacks (DNS rebinding, cross-site requests).
- Lifecycle. Hosted by the window means no tools while it is closed; a background service is a
  new thing to install, update and elevate correctly. Per-session processes start on demand.
- One stuck server affects every session.
- More to build and review: HTTP transport, token, Claude Code setup by URL, start/stop, and
  another safety review of what AI tools can read.

## Middle option, if memory becomes the problem

Keep a thin stdio process per session, but have it forward to one headless Sprawl over the same
authenticated named pipe the window uses. Keeps the account and elevation check, shares scans.
Process count stays the same, but process count isn't the cost that matters.

## Trigger to revisit

Sprawl `--mcp` processes holding large memory from repeated or duplicate scans.
