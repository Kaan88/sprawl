# Sprawl 1.0 design and product review

19 November 2026 · GPT Astra · Release 1.0.0 at `3452226`

Codex's proposed sequencing of these suggestions is in the
[alternative roadmap](alternative-roadmap-astra.md). It incorporates the user's requirement
to improve the existing native app and records rejection of all earlier Codex mockups.

Sprawl covers the core Windows disk-analysis job. The best next investment is making the existing data easier to interpret. On a 113.0 MiB folder scan, the most prominent figures described a 929.1 GiB volume. On the same scan the Advisor found 101.0 MiB of rebuildable output, yet two AI setup prompts competed with that single finding. The proposed design brings the scanned folder, its largest contents and the consequences of a selected action into view first.

## What I inspected

I opened the packaged executable twice with its screenshot mode, an isolated offline preferences directory and settings writes disabled. At 1280 × 800 I inspected Tree, File, Search, Advisor, Cleaners, Settings, a selected finding and its pre-execution review. At 1120 × 720 I inspected the first-launch workspace. Both windows were closed. The fixture contained 7 files and 15 folders. The large and small captures were separate runs with different states. The notes below reflect the live windows, independently of the older repository screenshots.

The fixture scan finished in 0.04 seconds. Loading recommendations come from the scan HUD in `ui.rs`. Cleaners displayed its unavailable state. A populated Cleaners catalog, large-result performance, live AI, display scaling, touch, high contrast and screen-reader behavior were outside this sample. No cleanup ran. The late-stage screen is a review of one local build folder, so it establishes the transition and permanent-deletion disclosure for that operation only.

## Changes in priority order

| Priority | Observation | Proposed change and test |
|---|---|---|
| 1 · Scope | The scan picker displayed C:, Selection named the fixture, and the capacity bar described C:. A folder’s actual allocation first appeared in the table. The bright scan line persisted after completion. | Use the scanned root as the heading. Give it a size, count and completion time. Label volume capacity separately with its drive letter. Separate changing the next scan target from inspecting the current scan. Keep the existing auto-scan preference clear on the picker. A person should be able to identify the current root, next scan target and volume free space independently. |
| 1 · Review | With the AI pane open at 1280 × 800 the first finding barely fit above a tall empty tray. Both the fixture finding and the rules’ unexplained-folder prompt concerned the same scan. “Clean up…” opened a second step headed “Clean up 1 item, about 101.0 MiB?” | Lead with findings, owner/path, estimated gain and recovery cost. Keep AI available on demand. Use a compact selection footer with “Review selection”. In review, group by actual outcome, expose permanent deletion, Recycle Bin, commands or handoffs as applicable, and name the final action. Preserve all existing Keep, exclusion, privacy, selection and revalidation gates. Back returns focus to the selection. |
| 1 · Task-local feedback | The Cleaners fixture showed “Press Check” and a read failure at the same time. Its failure remained in the scan header after returning to Advisor. | Use distinct initial, checking, success-empty and failed states. Put the specific retry action beside the local failure. Crossing to Advisor should restore that page’s status, while the last scan remains identified. |
| 2 · Search | With Search selected the general Filter field remained alongside the separate query box. Six filter menus and five drive toggles preceded the results. The extension grid and treemap stayed visible. | Give Search one primary input and a labeled source selector that defaults to the current scope. Keep common filters and enabled chips visible. Put additional drives, advanced syntax and examples behind named controls. Show the optional map on request. Preserve Enter to results, Ctrl+Enter to reveal in Tree, saved queries and deliberate drive opt-in. A no-match state retains the query, scope and a removable filter. |
| 2 · First scan | At 1120 × 720, two blank panes and the map toolbar appeared before “Where did the space go?”; the drive cards continued below the initial viewport. | Give the initial workspace to drive and folder selection. After scanning starts, keep scope, activity, counts and Stop stable. Display a percentage when the scan has a meaningful denominator. Offer the technical throughput HUD as detail. |
| 2 · Dense workspaces | At 1280 the root row elided the folder name, long file paths shared little space, and nested map labels overlapped on the dominant tile. Settings already had a clear section rail and footer, but several paragraphs separated adjacent controls. | Keep sortable, compact tables and aligned numbers. Preserve the basename during path truncation and expose the full path on focus or demand. Draw only labels that fit their tile. Collapse the extension pane into a named control when space is tight. In Settings, retain section navigation and label immediate actions at the point of use alongside the existing Save model. |

## Look and motion

Use a quiet window backdrop, one contiguous reading surface and native control states. Move the version into About. Let the title-bar icon carry the existing Sprawl gradient. Reserve the system accent for focus, current destination and the primary command. Give file types a small stable palette, risk a word and symbol, and selected map regions a clear outline. Keep City, Galaxy and the existing 3D styling available as explicit choices. A flat treemap is the proposed starting point because it makes area and selection easier to compare. Backdrop tint and palette are discretionary choices. [Microsoft Mica guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)

Use system Segoe UI Variable with a short hierarchy. Try 24 effective pixels for the current task, 14 for controls and reading text, 12 for secondary metadata, plus right-aligned tabular sizes. Use a native focus indicator that survives selection and remains visible over chart colors. A snapped window should retain the path, size, current scope and primary command before adding secondary columns. [Microsoft typography guidance](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)

For the prototype, try a roughly 167 ms reveal for the details pane, immediate selection feedback across tree and map, and one brief transition from activity to completion. Keep row positions and keyboard focus stable during refresh. Honor the Windows animation preference and Sprawl’s map-animation switch. The reduced-motion state uses static selection borders, counts and status text, with no zoom, tilt, sweep or pulses. These are design targets. The review did not measure native animation timing. [Microsoft motion guidance](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion)

## Three mockups to decide on

1. **Explore** with a populated Tree/File workspace, a scoped folder header, volume details and linked table/map selection. Show the same shell before a scan, during activity and snapped narrow.
2. **Search** with an example result, a long path, source selection and a zero-result query. Show which controls disappear when space is tight and where keyboard focus moves after revealing a result.
3. **Advisor and review** with a finding selected by the user, the recoverability explanation and an explicit review action. Include the final review with Back and a clearly labeled execution control. Show the related Cleaners initial or failure state as a variant. The mockup must operate entirely on illustrative data.

The first design decision is whether a user can answer three questions without consulting help. What was scanned? What occupies the space? What will happen to this selection? The second is whether those answers survive narrowing the window and disabling animations.

## Feature completeness and possible additions

**Core scope is feature-complete for 1.0.** Users can scan, locate large files, inspect ownership and recoverability, compare against saved history, preview selected changes and inspect the outcome. The product already supports goal alternatives, duplicates, reviewed linking and compression, guided moves, local learning, approved recognisers, growth tools and optional AI. The review assessed coverage, not runtime correctness across every one of those workflows. [Release scope and deferrals](status.md)

1. **Most valuable addition — scan coverage details.** Open a small details sheet from the scanned-root heading or Search source. Reuse the recorded root, method, time, unreadable count and known source age. Distinguish configured exclusions from observed read failures and from additional drives the user has never included. An error count alone cannot supply a list of skipped paths. Show only what that particular scan recorded, and mark unavailable evidence in older snapshots. This helps answer “Why can’t I find this folder?” using local data.
2. **Optional AI enhancement — evidence links in answers.** Pilot one question, “What grew since my last scan?” Attach each numerical claim to the corresponding folder and dated before/after observations. Selecting the citation opens the existing comparison. Show an unavailable baseline or reduced snapshot detail explicitly. Use deterministic local totals, label inferred explanations, and retain provider opt-in and the existing execution boundary. The incremental feature is verifiable navigation from prose to evidence. History, growth analysis, chat, finding evidence and reusable recognisers already exist.
3. **Conditional expansion — a shareable, path-redacted headless report.** The existing F5 proposal is worthwhile if users regularly need a support ticket or repeatable CLI output. Start with previewable totals, scope, coverage and a deliberate path-disclosure choice. Validate demand before prioritizing this ahead of the scope and review work. Existing MCP scan tools already serve connected clients.

The documented deferrals still stand, including the expanded accessibility and privacy-cleaner projects, broader launcher evidence, compression drift and splitting linked copies. This review does not reopen the release audit or authorize a new action engine.

## Codex mockup handoff

**Final user decision, 19 November:** all Codex mockups were rejected, including Gentle
polish, Clearer reading and Responsive feedback, because they did not resemble WinUI 3.
Both generated mockup files were deleted at the user's request. No proposal is approved
or awaiting review, and no application UI changes were implemented. The descriptions
below are historical records of rejected proposals, not planned implementation.

**User correction, 19 November:** the initial workspace-rail proposal below was rejected as
a redesign. The requested scope is improvements to the existing app. It is superseded by
three restrained options: **Gentle polish**, **Clearer reading**, and **Responsive feedback**.
All retain the top scan/selection cards, horizontal view tabs, adjacent tree and extension
tables, and map below. Existing colorful/depth map identity and City/Galaxy choices remain.
The illustrative map geometry is a simplified preview, not an exact screenshot recreation.

Gentle polish refines control alignment, spacing and borders. Clearer reading increases row
and heading legibility and slightly favors the folder table. Responsive feedback adds linked
selection emphasis, brief optional tile movement, and quiet completed-scan feedback. All
show explicit folder/volume labels. The preview supports local filtering, linked row/map
selection, simulated scan/stop and a compact Advisor selection/review explanation. No files
are read or changed. File View, Cleaners and Search labels preserve their location but those
workflows are not mocked in this revision. Review the three alternatives before app changes.

The initial proposal and its verification limits are retained below as history.

Codex prepared an interactive design proposal after this review. It covers the scoped Explore
header and linked table/map selection, a single Search input with an empty-results state,
findings-first Advisor with an explicit review dialog, and task-local Cleaners check/retry
states. The coverage sheet and AI evidence link are proposed additions, using illustrative
data. No application UI implementation is included.

This first proposal uses a compact workspace rail, a flat map and native-style controls.
Density and detail-motion options can be compared. Search uses the current sample scan;
saved-source selection, full Tree/File modes, Welcome and mixed-operation review remain
follow-up mockup coverage before those flows are implemented. The preview's scan and cleanup
controls simulate states only. They do not access files or call providers.

JavaScript syntax was checked. A local browser preview could not be inspected: localhost
navigation timed out, and file navigation was blocked by browser policy. Therefore native
rendering, responsive layout and keyboard behavior are not claimed as visually verified.
The next gate is user review of the proposal, followed by any requested revisions.
