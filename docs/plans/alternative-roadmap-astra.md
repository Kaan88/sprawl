# Alternative roadmap from Astra's review

19 November 2026 · Proposed by Codex from [Astra's design and product review](design-review-1.0.md).

This orders Astra's suggestions by practical value: correct interpretation and clear actions
first, everyday usability next, visual refinement after that, then optional expansion. The
grouping and sequence are Codex's recommendation, not a second Astra review. All items below
are proposed work; none is implemented by this document.

The core app is feature-complete for 1.0 in Astra's assessment. This is an improvement track,
separate from the older feature backlog and the completed release audit.

## Boundaries

- Improve the existing native WinUI 3 app. Retain its top scan/selection cards, horizontal
  navigation, tree/extension arrangement and map beneath the tables.
- Preserve the app's colorful map identity, depth options, City and Galaxy. Astra's suggestion
  of a flat default is a discretionary design idea, not an agreed change.
- All previous Codex mockups were rejected and deleted. They are not implementation references.
  For visible changes, present small, faithful before/after mockups of the affected native
  area first. No new navigation model, replacement shell or broad redesign is authorized.
- Preserve explicit selection, Keep/exclusions, privacy choices, cancellation, revalidation
  and final confirmation. AI cannot choose or execute cleanup on the user's behalf.

## A. Highest priority: clarify what the app is doing

These address misunderstandings in the existing workflow and should lead the next round.

| Order | Work | Why it matters | Completion target |
|---|---|---|---|
| A1 | Clarify scan, selection and volume scope | A small folder scan currently makes whole-drive figures prominent, and the next scan target can be mistaken for the displayed result. | Within the existing header, label the scanned root, current selection and volume capacity independently; show the folder's own size/counts and scan time. Changing the next target must not relabel the displayed result. Keep auto-scan behavior explicit. |
| A2 | Make Advisor selection and review clearer | Findings compete with AI setup prompts and an oversized empty tray; the first action's wording obscures the review step. | Keep findings and recovery costs visible; consolidate AI setup and make AI available on demand in its existing area. Use a compact selection tray and **Review selection** for the first step. Final review states permanent deletion, Recycle Bin, command or handoff as applicable, with Back and a clear final action. |
| A3 | Fix task-local status and failure states | Cleaners can show both an initial instruction and a failure; an old error can follow the user to another tab. | Distinct initial, checking, empty-success and failed states. Retry belongs beside the failing task. Switching tabs restores the correct local status while retaining the last scan's identity. |

Validate A1 with a folder scan on a much larger volume, A2 with different actual remedy
outcomes, and A3 by switching tabs after a failed check. Mockup review alone does not prove
the state transitions correct.

## B. High usefulness: make existing data easier to use

Scan coverage is Astra's strongest new-feature suggestion. The remaining items remove
friction from workflows the app already supports.

| Order | Work | Why it matters | Completion target |
|---|---|---|---|
| B1 | Add scan coverage details | Explains missing folders or stale results without another scan or an AI answer. | A small native details surface from the existing scan/source area: recorded root, method, time, unreadable count and source age. Distinguish configured exclusions, observed failures and drives never included. Older snapshots show unavailable information honestly; counts must not become invented skipped-path lists. |
| B2 | Simplify Search in its current tab | Two text inputs and many controls compete with results. | One primary query input; clearly labeled source; common filters and active chips visible. Put advanced syntax, examples and additional-drive choices behind named controls. Keep optional map access, saved queries, deliberate drive opt-in, Enter to results and Ctrl+Enter to reveal in Tree. No-match results retain query and scope. |
| B3 | Improve the first scan and scan activity | Blank analysis panes consume the initial viewport before source selection; activity feedback needs a stable place. | Use the existing workspace's empty state to foreground drive/folder choices. During scanning, keep scope, counts and Stop stable. Use percentages only with a meaningful denominator; technical throughput remains available as detail. |
| B4 | Improve dense tables, paths and labels | Truncated basenames and overlapping map labels make the core analysis harder to read. | Keep compact sortable tables and aligned numbers. Preserve basenames when shortening paths and expose full paths on focus/demand. Map labels appear only when they fit. In narrow windows, preserve path, size, scope and primary action; any extension-pane collapse must have an explicit restore control. |

Build B1 on A1's scope definitions. Search should consume the same source/coverage facts
rather than introduce another interpretation of scan scope. Check B2–B4 with long paths,
zero results and narrow windows, as well as populated normal-width views.

## C. Useful polish: refine the native look and feel

Do this within the established layout, after the functional priorities. It should make the
current app feel more consistent and responsive, with no loss of information or identity.

| Order | Work | Why it matters | Completion target |
|---|---|---|---|
| C1 | Native control, typography and spacing consistency | Small inconsistencies can make an otherwise native app feel unfinished. | Use the app's actual WinUI controls, system text styles, coherent alignment and restrained surface/border treatment. Right-align tabular sizes; keep focus indicators visible. Consolidate the oversized version presentation into About if approved. In Settings, retain its rail and Save model while labeling immediate actions at the point of use. Colors and backdrop changes are optional, separately reviewable choices. |
| C2 | Purposeful motion and selection feedback | Immediate linked selection and stable refreshes help users follow their actions. | Keep tree/map selection synchronized and row/focus positions stable. Try a brief detail reveal and one activity-to-completion transition; roughly 167 ms is a starting experiment, not a fixed requirement. Honor Windows and Sprawl motion preferences. Reduced motion uses static status and selection, without pulses, sweep, tilt or zoom. |

Basic contrast, keyboard operation, scaling and reduced motion are acceptance checks throughout,
not work postponed until C. Astra did not independently verify high contrast, screen readers,
touch, scaling, large-result performance or native animation timing in its sample. The broader
previously deferred accessibility project is not silently added to this track.

## D. Optional expansion: useful only after the foundations

| Order | Work | Why it matters | Completion target |
|---|---|---|---|
| D1 | Evidence links in AI growth answers | Makes AI claims verifiable using data the app already holds. | Start with **What grew since my last scan?** Each numeric claim links to its folder and dated before/after observations in the existing comparison. Compute totals locally, label inferred causes, and disclose missing baselines or reduced snapshot detail. Keep provider opt-in and the existing execution boundary. |
| D2 | Shareable, path-redacted headless report | Useful for support or repeated CLI reporting, but serves a narrower audience than the usability improvements. | Proceed if there is actual demand. Reuse the existing F5 proposal; expose scope, coverage, totals and a deliberate path-disclosure choice. Preview redaction before sharing. Existing MCP tools do not complete this separate feature. |

D1 depends on reliable scope/coverage and usable comparison navigation, not on finishing
every cosmetic change. D2 can be scheduled separately if a concrete reporting need appears.
Neither requires expanding the cleanup action engine.

## Delivery and review

Work in the order above, keeping each item independently reviewable. Before changing an
affected UI area, show a faithful, localized before/after proposal. After approval, implement
the agreed change and verify the relevant states in the actual app, using isolated fixtures
and settings. Avoid real cleanup, provider calls or installed updater changes as incidental
verification.

If independent review is requested for implementation, use one Astra pass at medium effort,
following the user's standing preference. Record fixes and unresolved limitations in the
retained review notes; the 1.0 safety approval does not approve new behavior automatically.

No UI proposal is currently approved. This document authorizes no implementation by itself.
Compression drift, Separate again, broader recognisers, History deltas, optional MCP response
formatting and previously deferred privacy work remain in the [existing backlog](status.md).
