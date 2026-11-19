# The Advisor: where it could go next

Written 2026-11-18, after 0.3.0. These original ideas remain design notes, not a current backlog.
By 1.0.0, supported usage evidence, recognition, duplicate hints/linking, goal alternatives,
learning, prevention and approved AI recognisers have shipped. Broader launcher/media support
and other extensions remain partial or future work. See [current status](status.md) for the
supported boundaries, completed Astra review and remaining verification work.
They are ordered by how much they would change what the Advisor is worth to someone, not by
effort. Every one keeps the rules the Advisor has today: findings pass the safety checks,
nothing is ticked for the user, the review comes first, and what is only a guess is shown as
information.

## What the Advisor is today, and its limit

It answers "what can go?" with rules about places: this folder is a cache, that one is a build
output, this application's folder has no application any more. Since 0.3.0 it also says "this
is big and old" on drives where no rule applies. Its limit is that it knows places, not the
person: it cannot tell a film watched once from a project archive, a game finished from a game
paused, or a download kept on purpose from one forgotten. The ideas below are ways to learn
that without guessing.

## 1. Evidence of use, not only of change

The weakest sentence the Advisor says is "not changed since". Windows keeps better evidence
than modified times, and some of it is already read:

- **Games:** Steam's `LastPlayed` is used. Epic, GOG Galaxy, Xbox/Game Pass, Ubisoft Connect,
  EA and Battle.net each keep a last-played time or a play log in their own files. Reading
  them turns "not changed since March 2025" into "last played March 2025, 63 GB". Which file
  each launcher keeps it in needs checking before building.
- **Programs:** Prefetch and UserAssist are used for applications; extend them to portable
  programs found on other drives (an exe in the folder, matched by name).
- **Documents and media:** the Recent items folder and jump lists name files that were opened,
  with dates. Used read-only, they can clear a folder ("opened last week") even when nothing
  in it changed. They cannot prove disuse, only use.
- **Last-access times:** where the volume reports them enabled, a last-access date newer than
  the modified date is real evidence; where they are disabled, say so and do not use them.

Result: each "old and big" row carries the strongest evidence available and names its source,
and rows with proof of recent use drop out.

## 2. Know what things are

A folder of 60 GB called `data` is not advice. Recognise kinds the way the Chromium recogniser
does, by layout:

- **A game install** (an engine's files, a launcher's manifest): offer its launcher's own
  uninstall, show the save folder separately and say it stays.
- **A game's leftover after uninstall** (mods, shader caches, crash dumps in the install
  folder): the game is gone from the launcher but the folder remains.
- **Emulator libraries and disc images:** often kept as an archive and its extracted copy.
- **Media libraries:** the same film in two qualities, a video and its source project, raw
  photos beside their exports.
- **Archives next to their extracted folders:** `x.zip` and `x\` with the same contents: one
  of them is spare. Cheap to detect (names and sizes from the archive's index) and one of the
  most common real wastes.
- **Installers of things that are installed**, in any folder, not only Downloads.
- **Virtual machines, phone backups, old Windows images.**

Each kind brings its own honest remedy text. Most stay information; a few (archive plus
extracted copy, installer of an installed program) are safe enough to offer as amber deletes.

## 3. Duplicates and near-duplicates, always on

Today duplicates are a button the user must press, limited by a read budget. Make the cheap
part automatic: same size and same name in two places is found from the scan alone, with no
reading. Show those as "probably the same" groups, largest first, and read to confirm only
when the user opens a group. Add folder-level duplicates (two folders with the same file names
and sizes): a copied game, a backup of a backup. With the linking remedy from
`future-features.md`, many of these become free to resolve.

## 4. A plan, not a list

People come with a need: "I need 100 GB on C:". The goal box picks from the safe items only.
Go further:

- Offer **ranked options** to reach the goal: "caches: 12 GB, nothing lost", "old build
  outputs: 40 GB, rebuild time", "move these three games to D: (it has 170 GB free): 180 GB",
  "compress Program Files: about 9 GB".
- **Moving** is the remedy the Advisor has no hands for. A guided move for whole folders that
  tools support (a launcher's own move command; user folders through Windows' own Location
  tab) would unlock the biggest items on a full system drive.
- Show the **cost** of each option in the same units: time to get back, risk, and what stops
  working meanwhile.

## 5. Learn from the person

The memory already keeps Keep and Not now. Use more of what the user does:

- What they cleaned before and never missed (the "usual" set) raises confidence for the same
  rule next time; what they always skip stops being shown first.
- "I still use this" on a row teaches the kind, not only the path: one answer about one game
  in a launcher's library can stand for "I manage my games myself".
- From history: what came back after cleaning. A cache that refills in three days is not worth
  cleaning; say so and stop suggesting it.

## 6. Prevention

Cleaning treats the symptom. Where a cache has a setting, say where: move the browser cache,
cap a launcher's shader cache, point `pip`, `npm` and `cargo` at another drive, turn on Storage
Sense for Temp, set Docker's disk limit. One line per finding, shown after a clean-up: "this
will be back; here is how to keep it small".

## 7. The AI where rules run out

The model is best at exactly the gap in section 2: naming an unknown folder. Use it there, and
make the result reusable:

- After the rules, hand the model the largest unexplained folders and let its answer become a
  **proposed recogniser** the user approves once, as cleaners are today, so the next scan
  needs no model.
- Let the user ask about a row in place ("what is this?") with the answer kept on the row.
- Keep the floor: a model's finding is never better than orange, never ticked.

## 8. Trust and proof

- **Show the evidence as a short list on every row**, each line with its source ("Steam: last
  played 2025-03-02"), so a wrong row is obviously wrong.
- **Undo** for the user's own files exists (the Recycle Bin). Say so on the result card and
  offer "Open the Recycle Bin" there.
- **A dry-run report** the user can save: what would go, why, and what was held back.

## Suggested order

1. Evidence of use for games and programs (section 1): it makes the new "old and big" rows
   trustworthy, and those rows are most of what a data drive shows.
2. Archive next to extracted copy, installers of installed programs, launcher leftovers
   (section 2): small, concrete, safe.
3. Cheap duplicates from the scan (section 3).
4. Ranked options for a goal, with moves (section 4).
5. Prevention lines (section 6).
6. Learning from the person (section 5), then the AI as a recogniser-maker (section 7).
