# Advisor improvements: plan

The AI advisor plan (`ai-advisor.md`) is built: phases 0 to 7. This plan covers what using it
showed: the AI feels slow and opaque, the findings stop at "copy this command", the Advisor
tab is one long page, and the map, which is Sprawl's strength, does little of the advisor's
work.

The rules from the first plan still hold. Rust finds, the model judges. Every claim is checked.
Commands come only from Rust. Every safety-relevant step gets an independent review: Opus at
xhigh effort while Fable is out of limit, and Fable's overall pass later
(`.claude/fable-review-queue.md`).

## Milestones

### 1. An AI that shows its work and can be stopped
- **Cancel:** a Stop button for "Ask" and for each chat question. The thread notices between
  steps. A Claude Code process tree is ended, and a run killed this way leaves no files behind.
  "New chat" stops the old thread's question, instead of letting it run to the end.
- **Cost and model on show:**
  - the Ask button says which model;
  - each result says its tokens in and out and, for the Anthropic API, an estimate in dollars;
  - Claude Code reports `total_cost_usd`, which is used as it is.
- **Last result kept:** the newest AI result is kept per scanned root (in
  `%LOCALAPPDATA%\Sprawl\advisor\runs\`). On the next scan of that root it is shown as "from 2 hours
  ago", with its findings checked again against the new scan. Asking again is optional, not
  needed.

### 2. A tidier Advisor tab
- **Sections:** Findings · Plan · Ask · Growth · Memory, as a row of buttons at the top. One
  section shows at a time, and the choice is remembered.
- **A summary strip at the top:** what can go now (green and amber with commands), what needs a
  decision (orange), and when the drive is full at the current rate.
- **Findings sorted and filtered:** by size, risk or owner; only those with something to run;
  only the AI's.

### 3. Streaming answers
- The Anthropic Messages API and OpenAI-compatible servers stream (SSE over WinHTTP, read as it
  arrives).
- The summary and chat replies build up live; tool steps show as they start.
- Claude Code: `--output-format stream-json` gives its steps and text as they happen, instead
  of tailing our own out file.

### 4. Act mode (behind confirmation)
- On a finding with a command, "Run now..." opens one confirmation that shows exactly what will
  happen: the size, the path, and the command in full.
- On yes, the command runs in a visible PowerShell window, the same text as the exported
  script, with the same `sprawl-rm` functions. "Send to Recycle Bin" is used where that is the
  remedy and the size allows.
- After it ends, the paths are scanned again and the result shows what was really freed.
  Done is recorded in memory.
- Never for red, never for a finding the safety checks held back, never without the confirmation.
  Orange needs a second, explicit "These are my own files and I want them gone".

### 5. The map does the advisor's work
- Right-click (or hover) a lit tile in the Advisor lens to show its finding's card on the map,
  with Open in the list.
- **Growth heatmap:** a lens that colours tiles by how much they grew since the growth baseline.
  It shows what grew at a glance.

### 6. History, the deferred work
- Save snapshots off the UI thread.
- Fix the cap bug.
- Delta snapshots, against the last full one: more points for growth at less cost.
- A light scan on a schedule (folders only, at log-on or daily, optional), so growth works
  without the user remembering to scan.

### 7. Settings and polish
- An AI settings section in the Advisor: provider, model, key and base URL, without the wizard.
- An empty state for "AI off": what it adds, and how to turn it on.
- Hand-off to the user's own uninstaller app: define what Sprawl passes (name, install folder,
  registry key, store id). It is launched when one is configured.

### 8. Tests that catch UI regressions
- Scripted screenshot runs for the Advisor flows: plan, keep, chat demo, duplicates, a filter
  from the chat, the AI result card from a recorded run. Their XAML is checked to load, and key
  texts are checked to appear.
- A recorded-response test of the agent loop for each provider: the JSON of a real run is
  replayed, with no network.

## Order

1, 2 and 4 are what felt awkward in use: start there. Then 3 (it makes 1 feel finished), 7,
5, 6 and 8. Each milestone is committed and pushed as it lands, and reviewed before the next
safety-relevant one starts.
