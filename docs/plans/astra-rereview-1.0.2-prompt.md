# Prompt for GPT Astra: re-review of the 1.0.2 fixes

You reviewed Sprawl at main 621418b. Your report is `docs/plans/astra-review-1.0.2.md`: 30
findings in six sections, each marked "changes requested". Each finding has now been fixed
on main in commits 1c5821a, 9e65d86, 77cca26, c9c8cd7, 078ed1c, 56d0b45 and dd4a5a4. Nothing
is released.

Please re-review **only these fixes**, not the rest of the codebase.

- The list to work from is the section "Fixes for Astra's 1.0.2 review" at the top of
  `.claude/fable-review-queue.md`. It has one item per finding, numbered as in your report
  (S1 #1 = section 1, finding 1), each with its commit, files, functions, tests, and what the
  author asks you to check.
- For each finding, read the change against your original text and say whether it is fixed.
  If it isn't, give the smallest acceptable remaining change, as before.
- Report any new problem a fix introduces, with its file and line.
- End with a verdict per section.

Points the author is least sure of:

1. **S5 #1 (Codex).**
   - Codex 0.170 has no setting that gives the model an empty tool set. Sprawl now checks
     before each run, with three Codex commands that send nothing:
     - `mcp list --json`: every server is switched off by name;
     - `features list`: the tool features are off;
     - `debug prompt-input`: the offered tools are within a fixed list.
     It refuses on any difference.
   - The fixed list still contains `exec`, which runs through the code-mode host (switched
     off), and the helper-agent tools.
   - Is "a checked setup, refused otherwise" an acceptable answer to "an enforceable empty
     tool/server set"?
   - Does a Codex home made for the check see every configuration layer that
     `exec --ignore-user-config` loads?
2. **S4 #5 (named streams).**
   - The stream list comes from the held handles, but the stream bytes are read by
     `path:stream` name.
   - Is that comparison sound while the handles are held?
3. **S4 #2 (privacy wording).**
   - Privacy texts may not use promise words such as "for good". Privacy sets are therefore
     named in the button's delete part ("Delete 1 for good and 1 privacy item") and in their
     heading ("deleted without the Recycle Bin").
   - Is this explicit enough?
4. **S3 #4 (pipe identity).**
   - Each message is read under ImpersonateNamedPipeClient at identification level.
   - Check the revert-failure path, which aborts the process.
5. **S1 #2 (legacy task).**
   - The task is now read through the Task Scheduler COM API, not as XML.
   - Check the ownership rules in `legacy_task_owned`.

Limits of the author's own testing:
- The test shell was elevated, so unelevated-only checks skipped themselves. Please run
  `cargo test --release --workspace --no-fail-fast` once from an unelevated shell.
- No real `codex exec` run was made, and no real npm/pip/NuGet clean-up.
- There are no screenshots of the changed review, Details and Markdown views.
- No cross-account pipe test was run.

Read-only: please don't push, release, register tasks, or run clean-ups.
