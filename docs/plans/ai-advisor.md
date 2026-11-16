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
| Claude Code | Both: DiskTree drives `claude -p`, and DiskTree is an MCP server for the user's own sessions |
| Web lookup | Allowed, but asks the user each time (allow once / allow for the session) |
| Reading small manifests | Allowed (`package.json`, Steam `.vdf`, README and similar, size-capped) |
| Snapshots | Measure the size first, then choose a retention policy |

## Design rules

1. **Rust finds, the model judges.** The model never sees the full tree, only a compact report
   plus tools to drill down.
2. **Every claim is checked.** A cited path must exist and its size must match, or the finding is
   dropped. A hard never-touch list can't be overridden by the model.
3. **Advise only.** Nothing is executed by DiskTree for now.
4. **Useful without AI.** The rules engine on its own already produces most findings. The model
   adds judgement, identifies unknown folders, plans, and answers questions.

## Architecture

```
crates/
  disktree-core       (exists) scan, MFT, tree
  disktree-advisor    NEW, no UI
    facts/      app attribution, last-used, projects, hidden space, cloud placeholders
    rules/      TOML rule packs + engine -> Findings
    safety/     never-list, path/size validation, in-use check (Restart Manager)
    snapshot/   compact per-scan store (only if the size test passes)
    memory/     prefs, decisions, label cache, approved rules  (%APPDATA%\DiskTree\advisor\)
    llm/        Provider trait + agent loop + tools
      anthropic.rs    Messages API over raw HTTP, claude-opus-5-5, adaptive thinking, streaming
      openai.rs       any OpenAI-compatible URL (OpenAI, OpenRouter, Ollama, LM Studio, vLLM)
      claude_code.rs  spawns `claude -p --output-format stream-json --mcp-config <ours>`
    plan.rs     goal mode (lowest-risk set of findings that reaches N GB), PowerShell export
  disktree            UI: Advisor panel, Opportunities treemap lens, chat
  disktree --mcp      stdio MCP server: talks to the running app over a named pipe,
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

### Phase 0: foundations
- **Snapshot size test:** write the compact format (name interning, parent index, size,
  allocated, mtime, zstd) for a real full-`C:` scan (about 3.3M items). The estimate is
  15–30 MB per scan; measure it, then choose a retention policy.
- `disktree-advisor` crate skeleton, the finding schema, the rule-pack format and the safety layer.

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
- Plain-English query to a DiskTree filter, highlighted on the treemap.
- Web lookup with consent, and manifest reading.
- Anomaly sense ("Chrome profile is 22 GB, typical is 1–3 GB").

### Phase 5: time (depends on the Phase 0 size result)
- Snapshots on every scan.
- Growth rates, "full in about N days", and "what grew since last week" explained.

### Phase 6: Claude Code integration
- `disktree --mcp` with the named-pipe bridge to the running app, plus a one-line
  `claude mcp add` setup shown in Settings.
- DiskTree drives `claude -p` with the same tools.

### Phase 7: advanced remedies
- Duplicates (size, then head/tail hash, then BLAKE3) with hardlink or block-clone suggestions.
- `compact /exe:lzx` savings estimate.
- OneDrive "always keep on this device" files that can be made online-only.
- Free space inside WSL/Docker disk images.
- Suggestions to move things to a drive with free space.
- A second-opinion verifier pass and an in-use check.

### Later
- Act mode: Recycle Bin and official commands only, behind confirmation.
- Live watching of file changes (USN journal).

## Dependencies on current work

This starts after step 4 of the WizTree-gap list. Settings persistence and the filter engine
from step 4 are prerequisites for phases 1 and 4.
