# Plan for 1.0.5

Planned on 2026-11-20 at the user's word: plan only, start later. Use one rustc job (`-j 1`) for
builds and tests.

## Status, 2026-11-20 (work begun; not committed)

- **A:** waiting for Astra's review of 1.0.4.
- **B, B2, C, D, E, F:** done, with tests. They are in `.claude/fable-review-queue.md` for review.
- **G2:** done differently from the plan below:
  - items meant for the Recycle Bin on a drive that has none (network, removable, UNC) are
    **held** with the reason, not deleted for good after a yes. That keeps the rule that an item
    meant for the bin is never deleted for good;
  - Stop gives up waiting after 5 s. The hang itself was not reproduced. (Changed 2026-11-21 after
    Fable's P2-1: the run is no longer ended while its worker acts; the item says it is finishing.)
- **G4:** a likely cause, fixed but not seen: the filter row lost its height in Search, where the
  filter box is hidden. Now it has MinHeight 32.
- **G5:** done:
  - the AI card header shortens the model name with an ellipsis and a tooltip;
  - the findings columns have minimum widths;
  - the AI setup rows fill the column with nothing cut off (screenshot checked).
- **G1:** open.
- **G3:** done:
  - the user's screenshot showed it: the start page (no scan) darkened its card with `#38000000`,
    while the panes and Search lighten with `#0DFFFFFF`;
  - the start page uses the pane fill now.
- **Colours and Tweaks:** done:
  - out of the Options menu, at the user's word (it acts on the treemap alone);
  - now a Tweaks button in the map bar beside Colour, shown only while the Treemap is on;
  - Ctrl+T still works.

- **J. Fable's four reports (2026-11-21):** every P1 and P2 fixed, with tests, except where the
  queue entry "Fixes for the four Fable reports" says not done (the update preflight before
  closing other processes; marking an AI's label in the Owner column). P3s are open.
  - The admin P1: Sprawl installs for all users in Program Files and runs elevated only from
    there; a Program Files copy updates by running the signed Setup. **Not run on a PC yet**
    (it would install into Program Files): test it on a spare PC or VM before the release.
  - Before release, also check: a release build (`cargo build --release`), the new dialogs
    ("Install for all users?", "Close the other Sprawl processes?", "Stop and close"), and
    Forget key in AI setup, with screenshots.

## A. Astra's review of 1.0.4

The request is `docs/plans/astra-review-1.0.4-prompt.md`, sections 1-7. Fix whatever it finds
first.

## B. Codex can propose recognisers and cleaners

**Problem.** Codex's chat gets no Sprawl tools, by design (`llm/agent.rs`: `Provider::Codex =>
false`). But Explain unexplained folders and the item's Ask button ask it to call `propose_rule`
and `propose_cleaner`. It can't, so it says so and points the user at "Ask", which is the same
Codex: a loop with no way out.

**Chosen fix (the user's pick).** Codex writes its proposals as JSON inside its answer, and Sprawl
replays them through the same checks a tool call goes through:

1. The no-tools chat prompt (Codex, and Claude Code without Sprawl's server) gains one
   instruction. When a proposal is wanted, end the answer with a fenced block tagged
   `sprawl-proposals` that holds `{"rules": [...], "cleaners": [...]}`, in the same shape as the
   `propose_rule` and `propose_cleaner` inputs in `llm/tools.rs`.
2. Sprawl takes that block out of the text it shows and parses it.
   - Each entry goes through `Tools::call("propose_rule" | "propose_cleaner", ...)`, so the
     validation is the existing one: the never-list, rules for information only or at least
     amber, no commands, privacy cleaners only from Sprawl. Nothing new is trusted.
   - Limits: 10 per answer, and only paths that appear in the report. Bad JSON is dropped, with
     a one-line note.
3. Under the answer, Sprawl says "N proposals waiting for you in Remembered", or which ones were
   refused and why.
4. The no-tools prompt stops telling the AI to point at the Ask button when Ask is the same
   no-tools AI.
5. Codex can't look inside folders, so its proposals rest on the report alone. The proposal card
   says "from the report only, not inspected".

Tests:
- block parsing (none, one, malformed, over the limit);
- a refused entry stays refused;
- the shown text has no block left in it.

## B2. Make it obvious that a proposal arrived (all providers)

The user didn't notice that the AI's proposals had worked. They only appear on a "Remembered (N)"
tab the user has to know about.

1. After any AI answer that queued proposals (from a tool call or a Codex block), show a clear
   strip under the answer:
   - "3 rules waiting for your approval": each rule's name and what it recognises, in one line;
   - a **Review and approve** button that switches to the Advisor's Remembered tab and scrolls
     to "Rules the AI proposes".
   Refused proposals are listed in the same strip with the reason.
2. The Remembered tab is easier to find:
   - a badge with the number of proposals waiting;
   - the tab title says "Remembered (3 to approve)" while any wait.
3. After Approve:
   - say what changed, for example "Approved: 4 folders (231 GB) are now recognised as PS3 disc
     dumps";
   - offer a button back to the findings, so the effect is visible.
4. Check it with screenshots: the strip, the badge, and the tab after the jump.
5. **No new check after each Approve** (the user's request, agreed).
   - Today `AdvApprove` (and `AdvUndo`) call `advisor_start()` straight away. That re-runs the
     whole analysis on the scan in memory (not a disk scan), once per click, while the user is
     still on the Remembered tab.
   - Instead, set a flag (`advisor.rules_changed`) and say "Approved. The findings are updated
     when you go back to them."
   - Run `advisor_start()` once, when the user leaves the Remembered tab: Findings, Cleaners, the
     main window, or closing the Advisor. Dismiss and Forget don't need a new run.
   - The confirmation in step 3 ("4 folders now recognised...") is shown after that one run.
6. **The run must never be lost** (the user's requirement: closing the app, a crash, anything).
   - An approved rule is saved at once, as today (`save_memory` in `AdvApprove`), so an approval
     itself is never lost.
   - The flag is saved too, in the memory file next to the rules, not only in memory. Write it
     before saying "Approved".
   - The flag is cleared only after a run **finishes** with the new rules, never when the run
     starts. A run that is stopped, fails, or dies with the app leaves it set.
   - The flag triggers a run on:
     - leaving the Remembered tab (any way out, including the main window's tabs and closing the
       Advisor);
     - the next start of the app, once a scan or a loaded scan is on screen;
     - a new scan, whose own analysis counts as the run and clears it.
   - Closing the app while it is set: nothing is needed at close, because the next start does
     the run. Don't block closing.
   - Tests (unit tests on the flag's lifecycle, with no window needed; put the logic in a small
     type that can be tested on its own):
     - five approvals and one leave give exactly one run;
     - Dismiss or Forget alone gives no run;
     - the flag is saved: approve, drop the state without leaving (a crash), load the memory
       again, and the start runs once;
     - a run stopped or failed halfway keeps the flag, and the next leave runs again;
     - a finished run clears it, and leaving again doesn't run;
     - a new scan clears it through its own analysis;
     - closing with the flag set, then starting, runs once.
   - Also check it in the real app: approve two rules, close Sprawl from the title bar, start it
     again; the findings show the new rules without a click.

## C. AI setup: presets and layout

1. **Preset dropdown beside Base URL** (OpenAI-compatible). Picking a preset fills the address;
   typing still works. Presets:
   - OpenAI `https://api.openai.com/v1`;
   - DeepSeek `https://api.deepseek.com/v1`;
   - OpenRouter `https://openrouter.ai/api/v1`;
   - Groq `https://api.groq.com/openai/v1`;
   - Mistral `https://api.mistral.ai/v1`;
   - Ollama `http://localhost:11434/v1`;
   - LM Studio `http://localhost:1234/v1`.

   Check each address against the provider's docs when building. The key stays per host, as now
   (`agent::openai_key`).
2. **Load models beside the Model box**, not beside Base URL.
3. Follow the UI rules: labelled dropdown rows, nothing clipped, and screenshots with the
   dropdown closed and open.

## D. Bug: Load models ignores a key that isn't saved yet

`llm/models.rs` `list()` reads the saved key (`config::key`, `agent::openai_key`), not the one in
the field. A new or changed key is refused until the settings are saved and opened again (the
user saw this with DeepSeek).

Fix: `list()` takes the key from the form (from `read_ai_fields`) when one is typed, and falls
back to the saved one otherwise. The host rule (the key goes only to the host it is for, over
https or to this machine) applies to the typed key too. Add a test.

## E. Last change inside a folder (Advisor)

The user asked for this in the Advisor's details: when anything under the folder last changed,
so they can judge whether it is safe to delete.

- Show "Last change inside: 3 Mar 2025 (8 months ago), in <relative path of that file>".
- Compute it for the open finding only, from the scan tree's `modified` times.
- Merge the existing helpers into one shared function: `confidence.rs` `newest`, `leftovers.rs`
  `newest_inside`, `projects.rs` `newest_inside` / `last_change`, and `stale.rs` `newest`.
- Say that it is the modified time, not last use; say "unknown" when no times were read.
- Maybe also show it in the unexplained-folders list.

## F. Stained Glass view is broken (since 1.0.0)

The user reports that the Stained Glass view scrolls sideways endlessly and very fast. It has done
this from the start and was never reported.

- Code: `crates/sprawl/src/views/packed.rs` `glass()`, a Voronoi treemap that relaxes every frame.
  The seeds move to their pane's middle and the weights adjust. The view is registered in
  `views/mod.rs` as `Kind::Glass`.
- Not diagnosed yet. Suspects:
  - seeds or weights drift without staying inside the panel, so the whole layout slides;
  - a pane's middle is computed in other coordinates than the seeds;
  - `made_for` never matches, so the layout is rebuilt each frame with a shifted start.
- Done when the panes settle and stay still within a few seconds, and stay still on a resize and
  on going into a pane. Check it on screen (capture a few frames a second apart and compare), and
  add a unit test: after N relaxation steps every seed is inside the panel and the total
  movement gets smaller.

## G. The user's reports, 2026-11-20 (not diagnosed yet)

1. **Open issue: Review sometimes does nothing after ticking.** Pressing Review selection with
   items ticked sometimes shows nothing. The user doesn't know when it happens, and it hasn't been
   reproduced. Left open; while working nearby, make any path that returns silently say why.
2. **Clean-up stuck on a network drive; Stop didn't cancel.**
   - Seen: duplicate PDFs on `Z:` (a network share), "to the Recycle Bin". The card stayed at
     "Cleaning up... 0 bytes freed so far" and Stop did nothing.
   - Likely: network shares have no Recycle Bin. The shell's recycle call then either waits on a
     hidden "delete permanently?" prompt or blocks on the network, and Stop is only checked
     between items.
   - Fix:
     - before the run, find network and removable targets (`GetDriveType`, UNC paths). Say "This
       drive has no Recycle Bin: these are deleted permanently", and require the user's yes in the
       review;
     - never let the shell show a prompt (no-UI flags);
     - run each item with a timeout, so Stop returns at once even while a call is still blocked
       (let the blocked call finish in the background; don't count it as freed).
   - Test with a fixture share (`\localhost\<share>`) if one can be made without admin rights;
     otherwise a unit test on the drive-type decision.
3. **The Tree and Files sections have a different background** from the other sections: the
   whole page, not only the tab button (check the tab buttons too). Make them all the same.
4. **Search moves up a few pixels** when switched to from another tab. Find the margin or header
   that differs, so every tab's content starts at the same height.
5. **Clipped at the right edge:**
   - the AI card's header row ("Ask deepseek-flash · OpenAI-compatible · deepseek-flash ·
     Change") cuts off "Change" with a long model name. Let the provider/model text shrink with
     an ellipsis and a tooltip, and keep the buttons whole;
   - in the same screenshot, the "Folders the rules haven't explained" text is cut off at the
     right ("...propose reusable information ru") instead of wrapping. Check the Advisor
     panel's width at narrow window sizes.

   Screenshot both at narrow and wide widths (the UI rule: nothing clipped).

## H. Light mode (the user's request, 2026-11-20): done, first version

- Built differently from the plan below, smaller:
  - `theme.rs` rewrites each XAML string in `state::load` when the theme is light;
  - greys and dark tints flip their lightness, coloured text is darkened, coloured fills stay;
  - the root gets a near-opaque light ground;
  - the caption buttons follow the theme (`AppWindowTitleBar.PreferredTheme`, new in the
    bindings);
  - the setting is Settings > General > Theme (Like Windows, Light, Dark), applied at the next
    start;
  - the map keeps its dark look.
- Checked on screenshots of the Tree view, the Advisor and the AI setup. The other pages were not
  looked at in light yet.
- The user also liked mockup 03 (underline tabs with counts) from
  `docs/mockups/designs-2026-11-20/`. It is built:
  - the page tabs are plain text with a 3 px accent underline (`look::Pick::Page`, style
    SprawlPage);
  - Advisor and Cleaners get count badges (`tab_counts`). The badges weren't seen on screen: the
    fixtures have no findings.

- The UI is dark only:
  - `RequestedTheme="Dark"` on Root;
  - `ApplicationTheme::Dark` in main.rs;
  - about 566 ARGB colours written into the XAML strings, across 15 files.
- Plan:
  - a theme setting (System, Light, Dark) in Settings > Look, System following Windows' app mode;
  - every written colour goes through a small palette (`look::c(Role)`) with a dark and a light
    value, roles like text, dim text, card, pane, border, accent, risk colours;
  - the map, the city and the views keep their own colours, but the map's background, captions and
    key follow the theme;
  - a theme change rebuilds the page, as the language of the XAML is strings.
- Check: screenshots of every tab in both themes, closed and open (the UI rule: nothing clipped,
  text readable).

## I. More than one custom AI (the user's request, 2026-11-20): done

- Built:
  - a key per host in Credential Manager;
  - named entries in `ai.toml`, listed in the Provider dropdown as "<name> (custom)";
  - a Name row and a Remove button in AI setup;
  - Save keeps or renames the entry.
- It is in the review queue as item 7 of the 1.0.5 entry.

- Today there is one OpenAI-compatible setup:
  - one `base_url` and model;
  - one saved key, bound to one host (`key_host`).
- A server without a key already works: the Authorization header is sent only when a key is saved.
- Plan:
  - named custom AIs in `ai.toml` (a list of name, address, model, effort), each listed in the
    Provider dropdown under its name;
  - each with its own key in Credential Manager (target named after the custom AI), still bound
    to its own host under the https rule;
  - Add, rename and remove in AI setup; the old single setup becomes the first entry on load.
- Key handling changes, so it goes to the review queue.

