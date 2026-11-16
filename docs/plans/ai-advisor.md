# AI Advisor: plan and roadmap

An on-demand advisor that analyses a scan and highlights where space can be won back, for the
user and other power users.

Decisions so far:

| Question | Decision |
|---|---|
| Audience | The user and power users: exact commands, real evidence |
| Act or advise | Advise only for now: findings, copyable commands, a PowerShell script the user runs |
| When it runs | On demand |
| Paths sent to the model | As they are (an outgoing-request log is viewable in the app) |
| Learning the user's choices | Yes: preferences, decisions, labels and approved rules are remembered |
| Claude Code | Both: Sprawl drives `claude -p`, and Sprawl is an MCP server for the user's own sessions |
| Web lookup | Allowed, but asks the user each time (allow once / allow for the session) |
| Reading small manifests | Allowed (`package.json`, Steam `.vdf`, README and similar, size-capped) |
| Snapshots | Measure the size first, then choose a retention policy |

## Design rules

1. **Rust finds, the model judges.** The model never sees the full tree, only a compact report
   plus tools to drill down.
2. **Every claim is checked.** A cited path must exist and its size must match, or the finding is
   dropped. A hard never-touch list can't be overridden by the model.
3. **Advise only.** Nothing is executed by Sprawl for now.
4. **Useful without AI.** The rules engine on its own already produces most findings. The model
   adds judgement, identifies unknown folders, plans, and answers questions.
5. **Commands come from Rust.** A model or a local rule pack names a remedy; only the built-in
   pack carries command text, and only it is trusted on risk (`safety::check`). Generated
   commands are quoted for PowerShell only: the UI must label them "PowerShell" (or launch them
   in PowerShell itself), since pasted into cmd.exe a folder named `a & b` would run `b`.
6. **Every new safety-relevant layer gets an independent review** before it counts as done.

## Architecture

```
crates/
  sprawl-core       (exists) scan, MFT, tree
  sprawl-advisor    NEW, no UI
    facts/      app attribution, last-used, projects, hidden space, cloud placeholders
    rules/      TOML rule packs + engine -> Findings
    safety/     never-list, path/size validation, in-use check (Restart Manager)
    snapshot/   compact per-scan store (only if the size test passes)
    memory/     prefs, decisions, label cache, approved rules  (%APPDATA%\Sprawl\advisor\)
    llm/        Provider trait + agent loop + tools
      anthropic.rs    Messages API over raw HTTP, claude-opus-5-5, adaptive thinking, streaming
      openai.rs       any OpenAI-compatible URL (OpenAI, OpenRouter, Ollama, LM Studio, vLLM)
      claude_code.rs  spawns `claude -p --output-format stream-json --mcp-config <ours>`
    plan.rs     goal mode (lowest-risk set of findings that reaches N GB), PowerShell export
  sprawl            UI: Advisor panel, Opportunities treemap lens, chat
  sprawl --mcp      stdio MCP server: talks to the running app over a named pipe,
                      or falls back to the latest snapshot
```

**Finding:** path, owner app, bytes, risk (green: regenerable / amber: costs something /
orange: user data / red: never), remedy, command, evidence[], confidence, source (rule id or model).

**Model tools:**
- Drilling down: `list_children`, `largest`, `by_type`, `owner_of`, `app_last_used`, `diff_since`
- `read_manifest`: small text files only
- `web_lookup`: asks first, showing the exact query
- `remember`, `propose_rule`
- `report_finding`: strict schema

**Providers** declare what they support (tool calling, structured output, context size). Weak
local models fall back to a single report with no tools. API keys are stored in Windows Credential
Manager.

## Roadmap

### Phase 0: foundations (done)
- **Snapshot size test:** write the compact format (name interning, parent index, size,
  allocated, mtime, zstd) for a real full-`C:` scan (about 3.3M items). The estimate is
  15–30 MB per scan; measure it, then choose a retention policy.
- `sprawl-advisor` crate skeleton, the finding schema, the rule-pack format and the safety layer.

**Measured** on a full `C:` folder walk (3,325,443 items, 1,353,402 distinct names) with
`cargo run --release -p sprawl-advisor --example snapshot_size -- C:\`:

| Format | Size | Write | Notes |
|---|---|---|---|
| CSV (WizTree-style) | 523.5 MB | | for comparison |
| Raw columns | 82.6 MB | 0.85 s | 24.8 bytes per item before compression |
| Full snapshot, zstd 3 | 29.2 MB | 0.9 s | |
| **Full snapshot, zstd 9** | **27.0 MB** | **1.2–1.4 s** | read back in 1.2 s, totals match |
| Full snapshot, zstd 19 | 24.3 MB | 14 s | not worth the time |
| **Folders only, zstd 9** | **4.6 MB** | 0.7 s | 820k nodes: folders plus one `<files>` total per folder; folder totals exact |

Names are most of the cost (about 40% of all names are unique: hashes, GUIDs, cache keys).

**Retention policy** (from these numbers):
- Every scan saves a **folders-only** snapshot (about 5 MB for a full system drive). Kept: all
  from the last 30 days, then one per week for 6 months, then one per month. That is at most
  about 60 per drive, around 300 MB for a drive scanned daily. These feed growth rates, the
  time-lapse and "what grew" at folder level.
- The **last 3 full** snapshots per scanned root are kept (about 27 MB each) for file-level
  "what changed since the last scan" and the scan-diff view.
- A total cap (default 1 GB, in Settings) thins the oldest folders-only snapshots first.
- Stored in `%LOCALAPPDATA%\Sprawl\snapshots\` (machine-local, not roaming).

**Built:** `crates/sprawl-advisor` with `finding` (schema), `rules` (TOML packs, `%VAR%`,
`*`/`?`/`**` matching, junctions never followed, a starter built-in pack of 11 rules),
`safety` (in the tree, still on disk, size within 1% or 1 MB, the never-touch list, no deletes
through junctions, the user's folders raised to orange; a model's finding is refused anywhere
on the never-list even as a "command") and `snapshot` (the format above). The
`examples/advise.rs` run over the same `C:` found 10 findings, none held back by safety.
The in-use check (Restart Manager) moves to Phase 1.

### Phase 1: Rust evidence (useful with no AI)
- Built-in rule pack:
  - dev caches (npm/pnpm/yarn, pip, Cargo, Gradle, NuGet)
  - browser and shader caches
  - Windows reclaimables (`%TEMP%`, crash dumps, Delivery Optimization, Windows Update
    leftovers, `Windows.old`, Recycle Bin)
  - old installers and ISOs in Downloads
  - WSL/Docker VHDX
- App attribution (Uninstall keys, AppX, Steam/Epic/GOG manifests, WSL/Docker) and last-used
  signals (Steam LastPlayed, Prefetch, UserAssist).
- Project detector (git, manifests, build-output-to-source ratio, last activity).
- Hidden-space accounting: used minus scanned, explained by VSS, restore points, reserved
  storage, pagefile, hiberfil, MFT and hardlink-aware WinSxS.
- UI: an Advisor panel (findings, evidence, copy-command), an Opportunities treemap lens and an
  Owner column in the tree.

### Phase 2: AI v1
- Providers: Anthropic, OpenAI-compatible URL and Claude Code `-p`.
- Report builder, agent loop and tools, structured findings with validation.
- Goal mode: "I need N GB" gives a lowest-risk plan, with an animated shrink preview on the
  treemap.
- PowerShell export with a dry-run section, and a verify rescan that measures how much space
  was actually freed.

### Phase 3: memory and learning
- Prefs, decisions and the path-pattern label cache.
- The `remember` tool.
- Accepted and rejected findings shape later suggestions.
- Approved rule proposals become local rule packs.

### Phase 4: conversation
- "Ask your disk" chat panel.
- Plain-English query to a Sprawl filter, highlighted on the treemap.
- Web lookup with consent, and manifest reading.
- Anomaly sense ("Chrome profile is 22 GB, typical is 1–3 GB").

### Phase 5: time (depends on the Phase 0 size result)
- Snapshots on every scan.
- Growth rates, "full in about N days", and "what grew since last week" explained.

### Phase 6: Claude Code integration
- `sprawl --mcp` with the named-pipe bridge to the running app, plus a one-line
  `claude mcp add` setup shown in Settings.
- Sprawl drives `claude -p` with the same tools.

### Phase 7: advanced remedies
- Duplicates (size, then head/tail hash, then BLAKE3) with hardlink or block-clone suggestions.
- `compact /exe:lzx` savings estimate.
- OneDrive "always keep on this device" files that can be made online-only.
- Free space inside WSL/Docker disk images.
- Suggestions to move things to a drive with free space.
- A second-opinion verifier pass and an in-use check.

### Later
- Act mode: Recycle Bin and official commands only, behind confirmation.
- Hand-off to the user's own uninstaller app, once it exists: an "Uninstall with ..." button
  that launches it with the app's identity (name, install folder, registry key, store id).
  Today Sprawl only opens Windows' Installed apps or Steam's own uninstall; it never runs an
  uninstall command from the registry, since any program running as the user can write one and
  Sprawl may run as administrator.
- Live watching of file changes (USN journal).

## Dependencies on current work

This starts after step 4 of the WizTree-gap list. Settings persistence and the filter engine
from step 4 are prerequisites for phases 1 and 4.
