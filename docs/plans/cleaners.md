# Plan: cleaners (a BleachBit-like mode, AI-assisted)

BleachBit has a list of cleaners, one per application. Each has options, a preview that shows
what would go and how much, then a clean. This plan gives Sprawl the same idea, fitted to what
Sprawl already is.

**Status (2026-11-17, end of day):** phases 0, 1, 2, 3 and 3b are built and pushed (commits
074804f, 098c56a, 6604067, 2f63c28, 3bcec86). Added on the user's request since: a Cleaners tab
(a449a42, the Advisor's Cleaners section alone), the Use/Scan buttons fix (959d7e3), and AI in
Cleaners (983a321: a chat card with ready-made questions; questions carry the cleaners text; the
`suggest_ticks` tool shows a card with Tick them, safe and amber parts only, review still first;
not yet tried against a real model). Next: phase 4 (carve-outs: browser caches and thumbcache
inside protected folders, all closed = must, with tests), then phase 5 (opt-in privacy cleaners:
Settings switch, the `Set` action, Chromium cookies and history, Firefox cookies, Recent and jump
lists). After that: one Fable review of all phases (notes per phase are queued in
`.claude/fable-review-queue.md`; 3b is not skippable), docs (README Search and Cleaners tabs and
Status, search.md and this file's status, CHANGELOG), version bump and release. Not yet tested: a
real clean-up run (the auto-mode classifier blocked it).

The rules from the earlier plans still hold. Rust finds, the model judges. Every claim is
checked. Commands come only from Rust. One clean-up path: ticks, the review that says "Deleted
for good", the handle-based executor. Every safety-relevant step gets an independent review.

## What exists today, and what it means for this plan

- **Rules** (`rules.rs`, `rules/builtin.toml`): one rule matches a path and gives one finding.
  About 30 built-in rules. Findings under 50 MB are skipped by default.
- **Actions** (`clean.rs`): `Remove`, `Clear { days }`, `Match { pattern }` and `Tool`. All
  deletes go through open handles and never follow a link. Tools run in a job object.
  The action is read back out of the PowerShell text Sprawl wrote (`clean::action`).
- **The never-list** (`safety.rs`) covers the browser profiles (`Chrome\User Data\**`, Firefox
  and Thunderbird profiles), `AppData\Local\Microsoft\**`, `AppData\Roaming\Microsoft\**` and
  `Packages\**`. This is why the Chrome and Edge cache rules are `info` today. Most of what
  BleachBit cleans in a browser sits inside the never-list.
- **Trust levels:** built-in rules are trusted on risk and may carry commands. A local pack's
  deletes are at least amber. A model's deletes are at least orange. Only green, built-in
  findings are ticked by Sprawl itself.
- **AI proposals** (`memory.rs`, `llm/tools.rs`): `propose_rule` exists. A proposal has no
  command, 1 to 8 narrow patterns, and a delete must end in a name. Approved ones go to
  `rules\local.toml` in the advisor folder.
- **In-use check** (`inuse.rs`): the Restart Manager says which programs hold files open
  (first 10,000 files). It is shown per finding. It does not block anything yet.
- **Registry** (`reg.rs`): read-only.
- **Gaps this plan must close first:**
  - `memory::local_packs` only parses a local pack. The narrow-pattern checks run when a rule
    is proposed and approved, not when a hand-edited file is loaded.
  - "Done" in memory hides a finding. A cache comes back, so a cleaner's part must not stay
    hidden after it was cleaned once.

## Main decisions

| Question | Decision |
|---|---|
| Where it lives | A **Cleaners** section in the Advisor, beside Findings. Not a new tab, not a new mode |
| A second clean-up path? | No. Cleaner parts are findings. Same ticks, same bar, same review, same executor |
| Format | The rule-pack TOML, extended: a cleaner is a named group of parts. Data, never code |
| Preview | The scan. Sizes and file counts come from the tree; nothing is walked again to preview |
| Space or privacy | Two classes. Space first. Privacy is opt-in, built-in only, in its own late phase |
| Registry items (MRU lists) | Out of scope here. Decided again after phase 5, in a plan of its own |
| Wiping, shredding | Never. Sprawl deletes; it does not overwrite |
| App running | Sprawl never closes an app. It checks, says which app, and waits for the user |
| Administrator | As now: no elevated helper. "Needs administrator" group and the restart button |
| AI | Proposes cleaners, finds leftovers, explains costs. It never adds delete power |
| AI off | Every built-in and hand-written cleaner works. Only the AI's three jobs are missing |

## What the user sees

### A normal user
1. The Advisor gets a section: **Findings · Cleaners · Ask · Growth · Remembered**.
2. Cleaners is a list of applications found on this PC, each with its total:
   `Firefox 1.4 GB`, `Discord 820 MB`, `Windows 6.1 GB`, `Developer tools 9.3 GB`.
   Applications that are not installed are not shown.
3. Opening one shows its **parts**, each with a tick box, a size, a risk dot with its word, and
   one line of cost: "Cache · 1.1 GB · Safe to delete (rebuilt when needed) · Pages load a
   little slower the first time".
4. A part's tick is the same tick as its row in Findings. Ticking in one place shows in the
   other. The bar at the bottom is the same bar: `7 selected · 12.3 GB  [Clean up...]`.
5. **Show files** on a part lists what would go and lights the tiles on the map. This is the
   preview. It needs no second walk, because the scan already has the files.
6. **Clean up...** opens the same review. It says "Deleted for good, not sent to the Recycle
   Bin". Parts are grouped under their application's name.
7. If an application is running, its parts show "Close Discord first" and can't be cleaned
   until it is closed. The review has a **Check again** button.
8. The result card and the clean-up log are the ones that exist. The log names the cleaner
   and the part.
9. A cleaner whose folders are outside the scan shows "Not in this scan" with a **Scan C:**
   button. Nothing is offered for a path the scan didn't see.

Sprawl ticks the same things as today: green, built-in, 1 MB or more. Nothing else.

### A power user
- **Small things show here.** The Findings list keeps its size thresholds. Cleaners shows a
  part from 1 MB (a part with nothing in it shows greyed, without a box).
- **Options per part:** "Keep the last N days" where the part allows it. The choice is kept.
- **My usual set:** Sprawl remembers which amber parts the user ticked last time and offers
  "Tick my usual ones". It never ticks them by itself.
- **Show the definition** on any cleaner: the TOML, the pack it came from, built-in or local.
- **Hand-written cleaners:** a `.toml` file in the advisor's `rules` folder. A file that
  doesn't pass the checks is listed with the reason, not skipped silently.
- **Proposals:** AI-proposed cleaners wait under Remembered, with what each one matches on
  this PC now. Approve, edit the title, or refuse.
- **Save as script** still works for parts that delete (the `sprawl-rm` functions).
- **MCP:** `sprawl --mcp` gets a read-only "list cleaners and their sizes" tool. No clean tool.

## The definition format

A cleaner lives in a rule pack. `[[rule]]` stays valid: a rule outside any cleaner shows only
in Findings. A part is a rule with a few more fields, so there is one matching engine.

```toml
[[cleaner]]
id = "app.discord"
name = "Discord"
group = "Chat"                      # Browsers | Chat | Games | Developer tools | Windows | Other
present = ['%APPDATA%\discord']     # shown only when one of these exists in the scan
running = ["Discord.exe"]           # process names that count as "the app is open"

  [[cleaner.part]]
  id = "cache"
  title = "Cache"
  class = "space"                   # space | privacy
  risk = "green"
  paths = ['%APPDATA%\discord\Cache', '%APPDATA%\discord\Code Cache', '%APPDATA%\discord\GPUCache']
  action = "clear"                  # clear | remove | match
  keep_days = 0                     # for clear: leave what changed in the last N days
  closed = "warn"                   # must | warn | no: what an open app means for this part
  cost = "Images and pages load again the first time."
  why = "Copies of things Discord downloaded. It fetches them again."

  [[cleaner.part]]
  id = "logs"
  title = "Old logs"
  class = "space"
  risk = "green"
  paths = ['%APPDATA%\discord\logs']
  action = "match"
  pattern = "*.log"
  keep_days = 7
  closed = "no"
  cost = "Nothing you would notice."
  why = "Text logs for troubleshooting."
```

What Sprawl needs that the rules don't have yet:
- **`action` as data.** `clear`, `remove`, `match` map straight to the executor's `Action`.
  `tool` (a command) is allowed in the built-in pack only, as today.
- **`present`:** the cleaner is listed only when the application is there. It can also name an
  installed application from the app facts (`installed = "Discord"`).
- **`running` and `closed`:** see the safety model.
- **`class`:** space or privacy. Decides the words, the group in the review and the defaults.
- **`cost`:** one plain sentence, shown on the part and under the review row.
- **`keep_days` as an option** the user may raise, never lower below the definition's value.

What was learnt from CleanerML, and what is not copied:
- Copied: a cleaner per application with options; a description and a warning per option;
  "is it running" by process name; several paths per option; preview before clean.
- Not copied: commands that edit files (SQLite vacuum, deleting keys from JSON or INI files,
  Chrome's autofill and favicons clean-up), `winreg`, deep scan by file name across the disk,
  shredding, and wiping free space. Each is a new kind of write. See "Left out".
- Not copied: a general variable language. Only `%VAR%` from a fixed list, `*`, `?` and `**`.

### Registry-backed items (MRU lists)
Out of scope in this plan. Reasons: they free no space, Sprawl's registry code is read-only,
and a registry delete is a new destructive kind with no handle-and-link model behind it.
Two related things stay off the table for good: Prefetch and UserAssist. Sprawl reads both to
say when a program was last used. Cleaning them would blind the Advisor.
The file half of "recent documents" (`...\Windows\Recent\*.lnk`, jump lists) is file-based
and is in phase 5. If registry items are wanted later, they get their own plan: a fixed,
built-in list of HKCU values, never a key path from a definition.

## Space cleaning and privacy cleaning

They look alike and are not. Space cleaning removes copies. Privacy cleaning removes the only
record of something.

| | Space | Privacy |
|---|---|---|
| Examples | caches, temp files, logs, crash dumps, thumbnails, update leftovers | cookies, history, recent-documents lists, download lists |
| Gain | disk space | almost none; the gain is that the trace is gone |
| Words | "Safe to delete (rebuilt when needed)", "Can be got back, takes time" | "Signs you out of every website", "Your browsing history is gone for good" |
| Lowest risk | green | amber, always |
| Who may define one | built-in, hand-written, AI-proposed | built-in only |
| Shown | always | only after "Show privacy cleaners" is turned on in Settings |
| Ticked by Sprawl | green built-in ones | never |
| "Select safe ones", "Select enough for N GB" | counted | never counted |
| "My usual set" | remembered | not remembered unless the user asks |
| Review | in the normal list | own group, "These free almost no space", own tick box |
| App open | per part | always `must` |

More on privacy:
- **Whole files only.** A cookie store goes as a whole, with its journal files, or not at all.
  Sprawl never opens a database to remove some rows.
- **What that rules out:** Firefox history (it shares `places.sqlite` with bookmarks), saved
  passwords and form data (never), and "cookies except these sites".
- **Honest words.** The part says: "Deleted, not overwritten. Recovery tools may still find
  it." Sprawl is not a wiping tool and must not sound like one.
- **Sync.** A browser that syncs may bring history back. The part says so.

## The safety model

### What stays as it is
- Every part becomes a finding and passes `safety::check`: in the tree, still on disk, size
  matches, not a link, no link inside, not the scan root, nothing from a CSV import.
- The check runs again before each item. The executor is `clean::run`. No new delete code
  path for space cleaners.
- Local and AI-proposed parts: never trusted on risk, at least amber, never ticked by Sprawl,
  no command text.

### The never-list and carve-outs
Most browser parts sit inside the never-list. The plan does not loosen the list. It adds a
second, smaller list beside it, in code, in `safety.rs`:
- **A carve-out** is an exact pattern below a never-list entry that a built-in part may
  clear: `...\Chrome\User Data\*\Cache`, `Code Cache`, `GPUCache`, the same for Edge and Brave,
  `...\Microsoft\Windows\Explorer\thumbcache_*.db`, and a few more.
- Only a part from the compiled-in pack can use one, and only when its matched path equals a
  carve-out pattern. A pack on disk can't name or add one.
- The action is `clear` or `match` on that folder. Never `remove` of a folder above it.
- Every other check still applies inside a carve-out: links, never-list names (a `.kdbx` in a
  cache folder stays), odd names.
- Profile folders that hold both (a `Default` folder with `Cache` beside `Login Data`) are
  never matched with `**`. Each carve-out names its last folder.
- A test lists every carve-out and fails if one could match `Login Data`, `Bookmarks`,
  `Local Storage`, `IndexedDB`, an extension's folder, a wallet or a key store.

### Applications running
- Two signals: the cleaner's `running` process names (this user's processes), and the Restart
  Manager on the part's files.
- `closed = "must"`: the part can't be ticked into a run while either signal fires. All
  carve-out parts inside a browser profile and all privacy parts are `must`.
- `closed = "warn"`: the review says "Discord is open: files in use will be skipped". The
  executor already carries on past a file it can't open.
- Checked when the review opens and again just before the item runs.
- Sprawl never ends a process. For browsers that stay in the tray it says how to quit them.
- A race is still possible (the app starts mid-run). The handle-based delete fails on files
  the app holds and reports them. For file sets, see below.

### File sets (privacy only)
A database and its journal (`Cookies`, `Cookies-journal`, `-wal`, `-shm`) must go together.
This is one new executor action, `Set`: open every file of the set for delete first; if one
can't be opened, none goes. It is the only new delete behaviour in this plan and is reviewed
as such.

### Administrator and other accounts
- No elevated helper, as now. Parts under `%SystemRoot%` or `%ProgramData%` show under "Needs
  administrator".
- Application cleaners act on the current user's profile only. Another account's folders show
  as information. This matches the existing rule for tool commands.

### What a definition on disk may and may not do
May: name 1 to 8 narrow path patterns per part, pick `clear`, `remove` or `match`, give a
pattern, days, a risk, texts, process names and `present` paths.

May not:
- carry a command, a registry path or a carve-out;
- be of class `privacy`;
- use a variable outside the fixed list, a `..`, a UNC path, or quoting characters;
- start with a wildcard, or have fewer than two named folders after the root;
- end a `remove` in a wildcard;
- name a never-list path, or a folder above one;
- claim green (it is raised to amber), or be ticked by Sprawl.

These checks run **every time a pack is loaded**, not only at approval. A pack that fails is
shown with the reason and none of its parts are offered. This also fixes today's gap.

### AI-proposed definitions: how they are checked and kept
1. The model calls `propose_cleaner` (it replaces `propose_rule`; old proposals still load).
   It can only name paths it saw through the tools in this scan.
2. Rust runs the checks above. A refusal goes back to the model as text, as today.
3. Rust runs the proposal against the current tree: **what it matches now**, the sizes, a
   sample of file names, and whether any match is the user's own files, a link, or near a
   never-list entry. A proposal that matches nothing on this PC is dropped.
4. The user sees the proposal with that list and approves or refuses. Approval is per cleaner.
5. Approved cleaners are written to `rules\local.toml` in the advisor folder, atomically, with
   the model's name and the date. They load as untrusted, like any file there.
6. From then on they are checked at every load and every finding passes `safety::check`,
   exactly like a hand-written one. Amber at least. Never ticked by Sprawl. Never in "Select
   enough for N GB".
7. **Forget** removes one. A cleaner whose application is gone for 90 days is offered for
   removal.
8. Any program running as the user can write to that folder. That is why local packs stay
   untrusted for good, however they got there. There is no "promote to trusted" button.
   Promotion means adding it to the built-in pack in the repository, with a review.

### What an Opus or Fable review must check
Per the project's gate: Fable reviews each safety-relevant phase; if Fable is out of limit,
an Opus xhigh review runs and the entry goes into `.claude/fable-review-queue.md`.
- No route from a pack on disk or from a model to: command text, a carve-out, a privacy part,
  a registry write, a green risk, or an automatic tick.
- Carve-outs: each pattern, read against the real folder layout of each browser. What else
  lives in a folder with that name. What a profile named oddly (`Profile 1`, `Guest Profile`,
  `System Profile`) does to the `*`.
- The load-time checks can't be skipped by a field older code ignores, a second file with the
  same id, a BOM, or a file over the size cap.
- The typed action on a finding and the PowerShell text for "Save as script" can't disagree.
- `running`: a process name match can't be spoofed into "closed"; a missing answer from the
  Restart Manager counts as open for `must` parts.
- `Set`: all or none; nothing goes when the app starts between the open and the delete.
- The preview shown at approval is the same set the executor would act on.
- Ticks shared between Findings and Cleaners: no path ticked in one view and hidden in the
  review; nested parts are not counted twice.
- Memory: a cleaned part comes back when it has grown again; Keep still hides it.
- Words: no privacy part says or implies "wiped" or "safe".

## The AI's part

Concrete jobs, all through the existing agent loop and tools:
1. **Recognise what the built-in list doesn't know.** During "Ask the AI to look further" the
   model gets the list of AppData and ProgramData folders that no cleaner and no installed
   application claims, with sizes. It uses `list_children`, `by_type`, `owner_of`,
   `read_manifest` and, with consent, `web_lookup` to say what each is.
2. **Leftovers of removed applications.** A folder whose application is no longer installed.
   The model reports it with `report_finding`. As today, a model's delete is orange: the user
   decides, one by one.
3. **Propose a cleaner** for anything that will come back (`propose_cleaner`), as above.
4. **Explain a cost in plain words.** "What happens if I clean this?" on any part. The answer
   uses the part's `cost`, the app facts (last used, size, growth) and the scan. It is text
   only, marked AI. It can't change a risk, a tick or a definition.
5. **A second opinion on a proposal** before the user sees it: a fresh, short verifier pass
   that is asked only "what could this delete that the user would miss?". Its answer is shown
   on the proposal.

What the AI never does: write a command or a carve-out, propose a privacy cleaner, read a
browser's data files, tick anything, or start a clean-up.

**When AI is off:** the Cleaners section is complete for built-in and hand-written cleaners.
In place of the AI there is one Rust-only helper: "Other apps (found by shape)". It lists
folders named `Cache`, `Code Cache` or `GPUCache` directly under an unknown application's
folder in AppData, as amber, unticked, named after that folder. The proposals area shows what
the AI would add and how to turn it on.

## Milestones

Each one: build, tests, screenshots on a real profile, commit and push, and the review named.
Only one cargo runs at a time.

### 0. Groundwork (no new feature)
- A finding carries its typed action. The PowerShell text is written from it, for the script
  only. `clean::action` stops parsing text.
- Local packs are checked at every load; failures are listed in Remembered.
- Memory: "Done" on a regrowing part hides it only until it is over its threshold again.
- Checks: the existing clean-up tests pass unchanged; a test that the typed action and the
  script text agree for every built-in rule; hand-edited bad packs are refused.
- Review: Opus xhigh, queued for Fable. This touches the layer that deletes.

### 1. Cleaners, space only, outside the never-list
- The `[[cleaner]]` format and parser. The built-in rules regrouped into cleaners (Developer
  tools, Windows, Games, Firefox cache, graphics caches).
- New built-in cleaners for common applications whose data is outside the never-list:
  Discord, Slack, Spotify, Zoom, Steam, Epic, VS Code, JetBrains, Adobe caches, and the
  "found by shape" helper.
- The Cleaners section: list, parts, sizes, Show files with map highlight, shared ticks,
  "Not in this scan".
- Checks: a fixture tree per cleaner; ticking in either section gives the same review; no new
  path into `clean::run`.
- Review: Opus xhigh on the format and the shared ticks.

### 2. Running applications, options, hand-written cleaners
- `running` and `closed`; the block in the review with Check again.
- "Keep the last N days" per part; "My usual set".
- Show the definition; the list of refused files; the read-only MCP tool.
- Checks: a test process holding a file blocks a `must` part and is skipped past for `warn`.
- Review: Opus xhigh.

### 3. The AI
- `propose_cleaner`, the match-now preview at approval, the verifier pass, Forget.
- The unclaimed-folders list in the report; leftovers of removed applications.
- "What happens if I clean this?".
- Checks: recorded-response tests per provider; proposals that try a command, a wide pattern,
  a never-list path or a privacy class are refused; an approved cleaner survives a restart and
  is still amber.
- Review: Fable (AI code), or Opus xhigh and the queue.

### 4. Carve-outs: caches inside protected folders
- The carve-out list in `safety.rs` and its tests.
- Chrome, Edge and Brave caches become cleanable (today `info`). Thumbnails. Windows logs
  that are safe to clear, as administrator.
- All `closed = "must"`.
- Checks: on a real profile of each browser, before and after: sign-ins, bookmarks, extensions
  and open tabs are intact; only the cache folders shrank.
- Review: Fable, not skippable. This is the first change to what the never-list protects.

### 5. Privacy cleaners (opt-in)
- The Settings switch, the privacy group, the words, the separate tick box in the review.
- The `Set` action. Built-in parts: Chromium cookies and history, Firefox cookies, the Recent
  folder and jump lists.
- Checks: with the browser open, nothing goes; a set with one locked file is left whole; the
  browser starts clean afterwards and bookmarks and passwords are intact.
- Review: Fable, and one usability read of the words.

### 3b. Public definitions, downloaded automatically (the user's decision, 2026-11-17)
After phase 3, because it reuses everything phase 3 builds for untrusted proposals.
- **Sources:** winapp2.ini (MoscaDotTo/Winapp2) and BleachBit's community CleanerML
  (bleachbit/cleanerml, the `release` folder only). Fetched at run time from their GitHub
  repositories over HTTPS, at most once a week and when the user asks; never bundled in Sprawl's
  installer or repository, so their licences (CC BY-SA 4.0, unconfirmed, and GPLv3+) stay with
  their files. The source and licence are shown with each imported cleaner.
- **Converter:** each entry becomes a proposal in Sprawl's own format. Only what Sprawl's format
  can say survives: folders and file patterns under the fixed `%VAR%` list, `*`, `?`, `**`, the
  running-process check. Registry keys, `FileKey` with recursion over a whole drive, SQLite or
  JSON edits, shredding and anything in the never-list are dropped, each with a reason the user
  can read ("needs a registry write, which Sprawl doesn't do").
- **Trust:** every imported cleaner goes through the same checks as an AI proposal
  (`propose_cleaner`'s checks and the match-now preview) and stays untrusted for good: at least
  amber, never ticked automatically, never green, and marked "from winapp2" or "from BleachBit".
  A downloaded file can't add carve-outs or privacy classes.
- **The AI's part:** explain what an imported entry does and what it costs to get back, and
  point out entries that match nothing on this PC so the list stays short.
- **Updates:** a new download replaces the old import as a whole; a cleaner the user approved
  keeps its approval only while its paths are unchanged.
- **Privacy:** the download is a plain GET of a public file; nothing about the PC is sent.
- Checks: a corrupted or hostile file (path traversal, a never-list path, a pattern over a whole
  drive, a huge file) is refused whole or entry by entry, never partly applied; the converter's
  tests use fixed copies of both files.
- Review: Fable, not skippable (downloaded input that leads to deletion).

### 6. Decide on the rest
With five phases in use: registry MRU lists (own plan or dropped), and whether any of "Left
out" below is worth its risk.

## Wanted later (the user's note, 2026-11-17)

Three things below are left out of this plan, not dropped. The user wants them kept in view
for a later milestone, each with its own plan and its own safety review:

- **Registry cleaning** (MRU lists and the like). Needs a registry write with a backup and an
  undo, and a fixed list of keys; never Prefetch or UserAssist.
- **Secure wiping** (shredding files, wiping free space). Has to say plainly what it can't
  promise on SSDs and with copy-on-write or cloud-synced folders.
- **Editing browser databases** (selective cookies, Firefox history, autofill). Needs the
  browser closed, a backup of each file, and a check that the file is the format expected.

## Left out, on purpose

- **Wiping, shredding, wiping free space.** Not what Sprawl is, and not reliable on SSDs.
- **Registry cleaning** of any kind in this plan. Prefetch and UserAssist: never.
- **Editing files:** SQLite vacuum or row deletes, JSON and INI key removal. So no selective
  cookies, no Firefox history, no autofill clean-up.
- **Passwords and form data.** Never offered.
- **Closing applications for the user.**
- **An elevated helper, and cleaning other accounts.**
- **Scheduled or automatic cleaning.** Nothing runs without the review.
- **The Recycle Bin as a target.** Deferred in the usability pass; unchanged here.
- **Bundling CleanerML or winapp2.ini** in Sprawl's installer or repository. They are
  downloaded at run time instead (phase 3b).
- **A cleaner list of Sprawl's own on a server.** Definitions come from the build, the user,
  the user's own AI, or the two public projects in phase 3b.
- **Deep scan by file name** across the whole disk (`Thumbs.db`, `*.tmp` anywhere).
- **Store apps** (`AppData\Local\Packages\**`): stays on the never-list until looked at alone.
