# Current status and next work

Updated 2026-11-19, after the 1.0.1 corrections. This page is the current index: status,
known limitations, open questions and the backlog. The older plans keep their designs and
dated implementation notes (see the last section).

## Where things stand

- **2026-11-23, after 1.0.7 (newest first):** F7 history deltas, F8 registry cleaning, F9 secure
  wiping and F10 browser-database editing are merged into main, with the Settings focus fix; see
  [future-features.md](future-features.md) for what each does. F8 to F10 passed several rounds of
  Fable safety review. Next: the UI benchmark (plan-1.0.5.md section J, branch `bench/j`), then a
  release. The older entries below are history.
- **Released:** 1.0.1 (2026-11-19, at the user's word, before the Astra review), with the
  corrections below. The user's installed copy was replaced with the same build first.
  1.0.0 (tag `v1.0.0`, rebuilt in place by Codex) is described further down.
- **Review:** the 1.0.1 commits await a GPT Astra (or Fable) read. The self-contained report
  is `.claude/fable-review-queue.md`, section "1.0.1 corrections by Claude", items 1-14.
  Findings go into a 1.0.2.
- **Next:** the user wants both: review [Astra's alternative roadmap](alternative-roadmap-astra.md)
  together with the backlog below, then choose. UI changes still need approved mockups first.
  Claude's review of it and a proposed order: [roadmap-review-claude.md](roadmap-review-claude.md).
  Decided 2026-11-19: 1.0.2 is reliability only (Astra's code findings, runtime checks, History
  cap, a scan-speed check); UI proposals are real screenshots from a throwaway branch, merged
  only after the user approves. After 1.0.2 the first design release is the small batch
  (scope lines, Advisor quick wins, Cleaners states).

## 1.0.2 in progress (reliability only)

Local on main after v1.0.1, unreleased; review entry in the queue ("1.0.2 (reliability)").
- 28524a0: the Advisor and AI report no longer list the scanned folder itself as unexplained.
- b14d612: History's size limit now also removes older full snapshots (never a folder's newest).
- eaba378: `sprawl-release build` first runs a scan-speed guard; it fails on 1.0.0's code (27x).
- Astra's review of 1.0.1 (changes requested, 5 high, 8 medium, 2 low) is fixed in
  d164e19..18fd68e, awaiting Astra's re-read (queue: "1.0.2 fixes for Astra's 1.0.1 findings"):
  - The elevated update hand-off writes no file: it starts Sprawl through the desktop shell
    with the version as an argument, and the receiving copy checks it is unelevated.
  - The old alert task is matched by its real principal and deleted only by an unelevated copy.
  - Whole-folder bin moves use the per-item freshness check (change time) and obey Stop.
  - Nothing runs on a network path, whatever offered it.
  - Tree Delete re-reads skip and Keep lists per item. It waits for Check again (and the
    reverse), and Close or Exit stops it after the current item.
  - Skipped folders: no names or file types while scanning.
  - Network roots never get an MFT read.
  - Inventories list each folder through its own checked handle (same speed).
  - An unreadable skip list stops network scans too, and skips on mapped drives cover the share.
  - A known folder on this PC's own share stays protected.
  - Compressed-file measurement and pre-1970 times are fixed.
- Still to come: Astra's re-read of these, and the runtime checks below that need the user.

## 1.0.1 corrections (Claude, 2026-11-19)

The user reported 1.0.0 as slow ("indexing is slow") with critical bugs, and asked Claude to
review Codex's work since `e022822`. Every changed file in `e022822..fed28fa` was read as a
diff. Targeted fixes were preferred over a revert.

| Commit | What 1.0.0 did wrong | Now |
|---|---|---|
| fa7ce20 | Pinned every folder's whole ancestor chain before listing it: non-MFT scans 15-55x slower. Refused network drives and shares. | Only the detailed walk pins per folder. Network roots are scanned read-only (no pins, WOF probe or MFT). |
| dbbadb9 | Opened every file for evidence of use, compressed-size measurement and the daily scan. | Listing metadata; the root is pinned once. |
| 39548e2 | Recycled folders file by file (50k bin entries, empty folders left). | Whole folder in one bin entry when nothing in it must stay. |
| c223324 | Refused to update while elevated: Always Run as Administrator copies never updated. | Hands off to an unelevated copy through explorer.exe. |
| da9573d | Delete in the tree ran on the window thread. | Runs on a worker with progress. |
| 081747d | History roots listed twice (old and v3 buckets). | Newest per root. |
| 0de55bf | A personal folder redirected to a share refused every delete. Reading exclusions blocked settings saves. | Share folders skipped in the protection list; plain read of settings.ini. |
| 678a4c2 | Compared file times exactly with whole-second snapshot times: manifests unreadable through Claude Code, archive copies lost for History scans. | Compared to the second; size still exact. |
| f71e6a7 | Left the pre-1.0 shared alert task running when elevated, and when turning alerts off. | Retired in both cases; the SID check stays. |
| ed3ce14 | Any skipped folder turned MFT scanning off. | MFT kept; skipped scopes dropped while the tree is built. |

Tests at d5b9d4a: app 66 passed / 1 ignored, advisor 200 / 5, core 60 / 1, installer 16.

## Known limitations

These are true of 1.0.1. Each is deliberate or accepted unless it is under "Open questions".

**Scanning**
- Folder inventories (non-MFT scans, search index, Cleaners reads, daily scan) don't see
  WOF compression hidden behind a plain listing; MFT scans and the detailed walk do.
  `compressed:` search says so.
- Network drives and shares are listed only: no MFT, no WOF check, no clean-up. MCP
  `scan_path` still refuses network paths.
- A folder swapped for a junction between its parent's listing and its own can be listed
  through the junction (wrong sizes in a read-only view; actions re-pin and check).
- With skipped folders, an MFT scan still reads the skipped records into memory. They are
  never shown or kept, but 1.0 never enumerated them. Live counts during the read include
  them; final counts don't. The live top-folder list stays empty during such a read.
  Accepted by the user (2026-11-19: "it's good").
- Snapshots keep whole seconds. A file rewritten at the same size within the same second
  passes the freshness check of `read_manifest` and archive-copy findings.

**Clean-up and safety**
- A file created inside a folder after the last check and before the whole-folder move goes
  to the Recycle Bin with it (undoable; a rename never follows links).
- Folders redirected to a network share have no local protection pattern. Nothing there can
  be cleaned: storage actions refuse non-local paths, mapped network letters included.
- Elevated tool commands may only start programs directly in System32 (79611d8). `dotnet
  nuget locals all --clear` is refused in an always-elevated Sprawl unless the override is on.
- Tool items measure the folder before and after with a detailed walk. If either walk has
  errors, no saving is credited and the item shows that as a problem, even when the tool
  succeeded.
- Per-file path pinning remains in `kinds::archives` (up to 50,000 matched files),
  `remedies::uncompress_scope` and `recycle::held` (one per `$I` record). Bounded or explicit;
  `held` is quick on the user's bin.
- Learning's "quiet" lasts until Forget learning. `exe_installers` reads up to 500 version
  resources per analysis. History name matching is case-sensitive (names come from disk).

**Updates and install**
- 1.0.0's own updater refuses to run elevated. 1.0.0 installs with Always Run as
  Administrator on can't auto-update to 1.0.1: turn the option off once, or run Setup. The
  CHANGELOG says so; the release notes must too.
- If an elevated Sprawl is still running, the unelevated updater's `stop_other_instances`
  can't stop it and says so.
- A leftover `.sprawl-recovery` that a matching version can't retire (mixed bundle, unknown
  files) makes the next update fail to prepare its recovery copy until it is removed.

**AI and MCP**
- `pipe::peer_is_us` compares the peer's token SID. A medium-integrity MCP client may be
  denied the token of an elevated app, so the in-app bridge may refuse an always-elevated
  Sprawl. Unverified.
- In-app MCP scans refuse while the app is elevated (or elevation is unknown).

## Open questions

Answered by the user on 2026-11-19: release 1.0.1 before the Astra review; reading skipped
folders' MFT records into memory is fine; after 1.0.1, review Astra's roadmap and the backlog
together. The user had no view on the three below, so they stay as they are until a reviewer
or real use gives a reason:

1. Keep the System32-only rule for elevated tool commands, or allow known toolchain paths
   such as `dotnet` in Program Files?
2. Should the bridge accept an elevated app when the token can't be read but the image path
   matches (the pre-1.0 check)? Needs a UAC test first.
3. Should a tool whose folder couldn't be measured show a note instead of a problem?

## Needs runtime verification

None of these has been run; each needs the user's consent (UAC, a real release, or a task):
- The elevated update hand-off end to end (needs a newer published release and a UAC prompt
  on restart). Watch the first update from 1.0.1.
- An elevated MFT scan of a drive with a skipped folder (the build is tested on a synthetic
  index only).
- The alert task migration with a real pre-1.0 task (none exists on this PC).
- The MCP bridge with an elevated app.
- Full-drive and elevated MFT performance after the 1.0.1 changes.
- From an elevated Sprawl (1.0.2): the unelevated launch through the desktop shell (Update now,
  and the old alert task's removal). Tested from an unelevated process only.
- Tree Delete: Close or File > Exit mid-Delete stops after the current item, then closes.
- A PC whose Documents (or another known folder) is redirected to a share on itself.

## Post-release scan regression correction (Codex, before 1.0.1)

The user reported slow/high-CPU C: indexing, stale Advisor results after drive changes and
slow Cleaners checks. Correction work takes priority over the alternative design roadmap.
The fixes separate fast directory inventory from detailed hidden-WOF probing, avoid a
whole-volume MFT read for Cleaner discovery, correct drive-picker routing, and isolate
reports/cancellation/title state during source changes. Fast inventory may omit hidden WOF
markers in compression search; detailed operation checks and MFT retain detection.

The user requested replacing the installed executable and rebuilding the GitHub 1.0.0
downloads/Setup in place. **Completed:** the installed executable now matches the replacement
bundle, with its original retained as backup. The rebuilt 1.0.0 release is published as latest;
all five GitHub asset SHA-256 digests and sizes match the local bundle. Its annotated tag
resolves to `0d8dc9af14ac6037a2838cb54612827eb7644547`.

One Astra medium verification pass requested additional scan-state fixes and approved them
after rereading. Debug app: 66 passed / 1 ignored. Release advisor: 200 / 5, core: 60 / 1,
installer: 16 / 0. Packaged and installed MCP initialize/list/ping each passed with 14 tools
and exit 0. The manifest signature and bundle hashes were checked. A disposable 2,000-file
benchmark measured about 127–133 ms wall / 1.1 s process CPU before, versus 12–13 ms wall /
0–16 ms CPU for inventory; CPU timer granularity applies. Full-drive/elevated MFT performance
and live UI drive switching remain unmeasured. Existing 1.0.0 users need the replacement Setup
because a same-version rebuild is not a version-based auto-update.

## Released and reviewed (1.0.0)

The agreed continuation of `next-round.md` is implemented and released in
[Sprawl 1.0.0](https://github.com/Kaan88/sprawl/releases/tag/v1.0.0), source/tag `3452226`.
GPT Astra reviewed the entire retained Fable dossier and the current source, requested
corrections, reread the fixes and approved at `26e0c76`. All 33 audit findings are marked
Fixed/Done in `.claude/fable-review-queue.md`. Astra requested no additional Fable review.

Shipped scope includes MSI/EXE/ZIP and supported Steam-layout recognition, duplicate hints
and checked linking, capacity-aware goal comparisons, guided Steam/Windows moves, prevention
advice, local learning and approved reusable recognisers. Decimal units, scan exclusions,
narrow tree layout and filesystem-compression search are implemented. Existing command
remedies, supported toolchain versions, compression, disk shrinking, History and scheduled
notifications remain subject to their documented owner/scope/confirmation checks.

The updater now stops other same-account processes running this exact installation before
replacement, excluding itself and waiting for exit. This includes lingering Sprawl MCP
servers. A verified peer that cannot be stopped prevents publication. This handles peers at
update time; it is not a new guarantee about an MCP host's process lifetime or reconnect policy.

Final verification: release advisor 200 passed / 5 ignored, core 59 / 1, installer 16 / 0;
debug app 64 / 1. The release compiled-version-floor test passed with a lower pretend
version. The packaged 1.0.0 executable passed read-only MCP initialize/list/ping and exited 0.
Program/installer versions, signed manifest and all five uploaded asset digests were checked.

## Newly requested design and product review

The user commissioned one GPT Astra review at medium effort on 2026-11-19, covering modern
WinUI 3 appearance, layout, animations, interaction feel and usability, core feature completeness
and worthwhile new features or AI enhancements. That review is complete:
[design and product review](design-review-1.0.md). Astra inspected the packaged release live at
1280 × 800 and a separate Welcome state at 1120 × 720, using isolated offline settings and a
scratch fixture. No cleanup ran. Loading motion and populated Cleaners/AI were not observed live.

Astra considers the core scan/find/understand/review/reclaim workflow feature-complete for 1.0.
Priorities are clearer scan scope, findings-first layout, explicit Review selection and local
error states. The most useful addition is a scan coverage sheet; optional AI work is navigable
evidence links for numerical claims. A path-redacted headless report is conditional on demand.

**Mockups first.** Proposed UI changes must be shown for review before application-code
implementation. The prior 1.0.0 safety approval does not approve a new design. Preserve
explicit selection, risk visibility, cancellation, Keep/exclusions and cleanup confirmation.
This task does not authorize running real cleanup or changing the user's installed settings.
The user rejected all Codex mockups on 19 November for not resembling WinUI 3. Both generated
mockup files were deleted as requested. No UI proposal is approved and no app UI changes
were implemented; the review's handoff records the rejected proposals as history.

The [alternative roadmap from Astra's suggestions](alternative-roadmap-astra.md) groups the
proposed improvements by usefulness: scope/review/status first, coverage and daily usability
next, native polish after that, then optional AI evidence links and reporting. It preserves
the existing app structure and records the mockup approval gate. It is a proposal, not started work.

## Remaining feature backlog

These are future work or deliberate scope limits, not unfinished 1.0.0 audit corrections.

| Item | Current boundary |
|---|---|
| Generic headless reports (F5) | `--report`, report formats, `--no-paths` and snapshot comparison are not implemented. MCP and scheduled `--check` do not complete this separate CLI workflow. |
| Compression drift | A dedicated "Compress again" workflow after application updates remains future work. Compression records, Undo and compression search already exist. |
| Separate linked copies | A dedicated "Separate again" action remains future work. Checked linking and its shared-edit warning are implemented. |
| More evidence and recognisers | Additional launcher last-played readers need representative data. Broader media, emulator and backup recognition remain ideas; supported MSI/EXE/ZIP/Steam behavior must not imply other formats are understood. |
| History deltas | On the roadmap as F7 (future-features.md). Snapshots are bounded full/reduced trees, not incremental delta snapshots. |
| Registry cleaning, secure wiping, browser-database editing | On the roadmap as F8, F9 and F10 (future-features.md), each with its own plan and safety review. |
| MCP presentation | Cleaners already return text with IDs, risk and counted bytes. Optional work is a structured row response and a `scan_report` overview of available scans. `list_scans` already exists. These extras were intentionally omitted from the earlier agreed scope. |
| Privacy and accessibility | Recent-items/jump-list/Firefox privacy cleanup and expanded screen-reader/theme work remain deliberately deferred under the earlier user decisions. Basic readable contrast, keyboard operation and reduced-motion behavior still matter in design review. |

Volta per-version removal, unsupported launcher moves and additional VM products are not
promised actions. Owner tools and reliable evidence are required before expanding support.

## Runtime and visual verification still needed (from 1.0.0)

Astra's completed audit was source-based; Sol ran the recorded tests. Installed update and
recovery launches, real scheduled-task changes, owner uninstall workflows, provider calls,
full-drive performance and some UI behavior have no new independent runtime verification.
Earlier black captures and WinUI teardown `0xC000041D` are recorded limits, not resolved by a
source approval. The newly commissioned visual review should distinguish current live
observations from historical screenshots and source-only conclusions.

## How to read the old plans

`next-round.md`, `future-features.md`, `advisor-next.md`, `advisor-improvements.md` and
`handoff-opus.md` retain their original designs and dated implementation notes. Their old
"awaiting Astra", "not built" or "stop before review" passages describe those checkpoints.
The 1.0.1 work is recorded in `next-round.md`'s Progress section and in the table above.
Use this page and the review dossier's final decision for current status. Do not erase the
historical evidence or turn a deliberately deferred idea into a release requirement.
