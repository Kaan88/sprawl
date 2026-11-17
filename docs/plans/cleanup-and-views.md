# Plan: clean-up inside Sprawl, and the new views

Two pieces of user feedback:
- Running PowerShell scripts outside the app is unwelcome. Exporting a plan as a script makes
  the Advisor hard to use.
- The flow "make a plan, ask the AI, pick from its suggestions, add them to the plan" is
  unintuitive.

The user also asked for every view in the View Lab gallery to be built. They will keep the
ones they like.

## A. Clean-up, one flow, inside Sprawl

### What the user sees
1. **One list of findings, with checkboxes.**
   - The rules' findings and the AI's findings are in the same list. AI findings carry an
     "AI" badge.
   - Green items that something can act on start ticked. Everything else starts unticked.
   - Nothing that can't be acted on has a box: information, moves, uninstalls.
2. **A bar at the bottom of the Advisor**, always visible:
   `5 selected · 12.3 GB   [Select safe ones] [Select enough for __ GB]   [Clean up…]`.
   It replaces the Plan section, the goal box, "Add to plan" and "Make a plan".
3. **Clean up… opens a review panel inside the Advisor, not a dialog box.**
   - It lists every ticked item with its size and what it costs.
   - Items that need administrator rights are grouped under "Needs administrator", with a
     button to restart Sprawl as administrator.
   - If any of the user's own files are included, a tick box is required: "These are my own
     files and I want them gone".
   - Buttons: **Clean up now** and **Back**.
4. **The clean-up runs inside the Advisor.**
   - Each row shows a spinner, then "freed 1.2 GB" or the reason it failed.
   - A total runs at the top. Stop finishes the current item and skips the rest.
5. **The end:** "Freed 11.8 GB" and a **Scan again** button. What went is remembered as done.
6. **Ask AI stays in its section.** Its findings go into the list straight away, unticked and
   marked AI. There is no separate step to choose from them.
7. **"Save as script…"** is a small link under the bar, for people who want a script. It is
   off the main path.

### How it runs
- **Deletes are done by Sprawl in Rust**, not by PowerShell. There are four kinds today:
  - `rm`: remove a folder or file;
  - `clear <days>`: what is inside, leaving anything changed in the last N days;
  - `match <pattern>`: matching files below a folder;
  - tool commands.

  They become a typed `Action` on the finding. The rules keep their PowerShell text, used for
  "Save as script" and for display.
- **The Rust delete has the same guarantees as `sprawl-rm`:**
  - It never follows a junction or symlink: the link itself is removed.
  - Names ending in a dot or space are skipped.
  - Read-only files are made writable first.
  - It refuses a root that is a reparse point.
  - Errors are collected per file, and it carries on with the rest.
  - It reports bytes freed as it goes.
- **Tool commands** (npm, yarn, pip, go, dotnet, conda, Clear-RecycleBin, DISM, Delivery
  Optimization) run as a hidden process.
  - Their output is captured and shown in the item's row.
  - They start in System32, with a fixed program path.
  - They run only under the user's own profile, as now.
- **Administrator items:** Sprawl never starts an elevated helper. When it isn't elevated, it
  offers "Restart as administrator" and leaves those items unticked.
- **Before each item:** `safety::check` runs again, the item is checked to still be in the
  current report, and the path must not have become a link.
- **Afterwards:** the paths are measured, Done is recorded unless the user chose Keep, and
  Scan again is offered.

### Order
1. `Action` in sprawl-advisor (parsed from the rules' commands and from model findings), with
   tests against the existing PowerShell behaviour.
2. The Rust executor (`clean.rs`) with tests on a temporary tree: links, read-only files, odd
   names, `days`, pattern, stop, errors.
3. The UI: checkboxes, the bar, the review panel, the run. The Plan section and the goal box
   go away; Export moves under "Save as script…".
4. An Opus xhigh review: this is the layer that deletes.

## B. The views (all 17 from the View Lab)

The renderer draws instanced boxes with labels. The views need a general layer first.

### Groundwork
- **A mesh layer:** coloured triangles and lines, in screen space for 2D views and in the
  existing 3D camera for 3D views.
  - Picking is a per-view hit test, so the hover HUD and clicks work in every view.
  - It reuses the label renderer for text.
- **Shared layouts:**
  - a radial or linear partition (Rings, Flame Stack);
  - circle packing (Suds);
  - a time series built from snapshots (Tideline, Tree Rings, Isobars, Rose Stack).
- **A view picker:** the three buttons become a strip with a "More views" menu. Each view
  remembers its own zoom.

### Views, in batches (each batch is committed with screenshots)
1. **2D partitions:** Rings (sunburst, click to zoom), Flame Stack, Waffle, Library.
2. **Packing and cells:** Suds (circle packing), Stained Glass (Voronoi treemap, relaxed over
   frames), Spectrum (extensions as a circular equaliser).
3. **Flows and strata:** Flow (Sankey: type, then folder, then risk), Strata (layers by risk).
4. **Time:** Tideline (streamgraph across snapshots), Tree Rings, Isobars, and Rose Stack (3D).
5. **Trees and 3D:** Grove (botanical tree), Lens (hyperbolic tree), Ridgeline (terrain mesh),
   Molecule (force-directed, 3D).

The colour switch (type, risk, owner, growth) applies to every view.

## Order between A and B
A comes first: it is what makes the app hard to use today. B's groundwork starts once A's
first two steps are in.
