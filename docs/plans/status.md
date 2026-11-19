# Current status and next work

Updated 2026-11-19 after Sprawl 1.0.0. This is the current index for the older design plans;
their dated progress entries remain an implementation history.

## Post-release scan regression correction

The user reported slow/high-CPU C: indexing, stale Advisor results after drive changes and
slow Cleaners checks. Correction work takes priority over the alternative design roadmap.
The fixes separate fast directory inventory from detailed hidden-WOF probing, avoid a
whole-volume MFT read for Cleaner discovery, correct drive-picker routing, and isolate
reports/cancellation/title state during source changes. Fast inventory may omit hidden WOF
markers in compression search; detailed operation checks and MFT retain detection.

The user requested replacing the installed executable and rebuilding the GitHub 1.0.0
downloads/Setup in place. Verification and publication status must be recorded separately
from the original release approval below.

## Released and reviewed

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
| History deltas | Snapshots are bounded full/reduced trees, not incremental delta snapshots. |
| MCP presentation | Cleaners already return text with IDs, risk and counted bytes. Optional work is a structured row response and a `scan_report` overview of available scans. `list_scans` already exists. These extras were intentionally omitted from the earlier agreed scope. |
| Privacy and accessibility | Recent-items/jump-list/Firefox privacy cleanup and expanded screen-reader/theme work remain deliberately deferred under the earlier user decisions. Basic readable contrast, keyboard operation and reduced-motion behavior still matter in design review. |

Volta per-version removal, unsupported launcher moves and additional VM products are not
promised actions. Owner tools and reliable evidence are required before expanding support.

## Runtime and visual verification still needed

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
Use this page and the review dossier's final decision for current status. Do not erase the
historical evidence or turn a deliberately deferred idea into a release requirement.
