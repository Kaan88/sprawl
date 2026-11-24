# Changelog

Each release of Sprawl is listed here, newest first. The section for a version is what the
in-app updater shows.

## 1.0.9 — 2026-11-24

- Search: after F2 renames a result of the scan, the results come back instead of the list staying on "Getting the names ready...", and the Advisor checks the renamed scan again instead of saying to scan a drive; Delete, Overwrite and rename wait while a `dc:` or `da:` search reads times (it holds those folders open), and a `dc:` or `da:` search started while a Delete, Overwrite or clean-up runs waits for it to finish and then runs by itself; Ctrl+C and Copy Path copy up to 100,000 picked results and say "pick fewer" above that instead of freezing; past 500 labels an AI label no longer pushes out a folder you hid.
- Tree View: Overwrite and delete from the map's right-click menu acts on the tile again when its row is folded away, as Delete does; typing a name to jump to another row, Left to the folder above and F4 (reset expansion) no longer leave picks the keyboard isn't on or can't see (Shift+Left picks the folder above).
- File View: Delete, Delete Permanently and Overwrite and delete from the map's right-click menu on a folder act on that folder again, asking about it with every check, as the menu's other items do.
- Tree View: a file selected in another tab and not on screen no longer joins the picks on a Ctrl+click or Shift+arrow, and Left or Right drops the picks when it moves to another row or folds a folder away.
- Search: a result listed by two saved scans is opened, shown and copied once, and counts once toward Open's limit.
- Delete: in File View, Delete and Overwrite and delete only take the files you picked (a folder left selected in Tree View is no longer asked about); Ctrl+click on a picked row now really takes it out; picks are dropped when you switch tab; a Delete still measuring other drives' folders stops when the result list changes; turning the Treemap off forgets a tile you right-clicked.
- Search: while `dc:` or `da:` reads the matches' times, the list is emptied and the status says it is reading, so the rows of the search before can't be picked and deleted meanwhile; a range that no read match falls in says "None of the 36,313 read is in the range; 35 couldn't be checked" instead of "Nothing could be checked"; and on a drive whose times are never read, the status gives that reason instead of "too many to read".
- Ask your disk: a folder's last-opened date comes from its listing, so the AI is told it can look older when its files are open right now or used under another hard-link name.
- Search: a size inside a group works wherever it stands (`<a <1gb b>`), `childcount:` and `childfilecount:` count a cloud folder such as OneDrive that was read, the help now says folders that could not be read or are excluded count as empty, and `x|file:<a b>` says to put `file:` or `!` in front of the whole word instead of dropping the group.
- Search and Ask your disk: when some matches have no last-opened or created time, each reason is counted on its own line (a folder with no files, an item gone since the scan, a link or a name Sprawl doesn't open) instead of one reason for all; a file under a deleted folder is called gone; a PC that keeps no last-access times says so before a search is called too big; files with long non-Latin names are read like any other.
- Search: `file:` and `folder:` in front of a word with a group (`file:*.tmp|<cache old>`, `folder:<a b>|c`) now set the kind instead of being searched as text, `<2024 report>|draft` says a group can't start with a number instead of quietly limiting sizes, and `childcount:0` / `childfilecount:0` no longer list junctions and symlinks (the scan never looks inside them) as empty folders; the help says `dupe:` and `sizedupe:` look within one scan before `dc:` and `da:`.
- Search: groups nest whatever comes first (`<<a b>|c d>`, `!<<a b>|c>`), a size at the end of a group (`<a <1gb>`) counts, text glued to a group's `>` (`<a b>>1gb`) is an error instead of being dropped, `!<dm:2026>` skips items with no date as `!dm:2026` does, and `len:` counts the path as it is on disk.
- Folders the rules haven't explained: a label on a folder that could never be listed (Users, AppData, ProgramData) no longer hides the folders under it; past 500 labels the oldest AI label goes before a folder you hid, and Hide always sticks; Hide works for a folder named like a %VARIABLE%; a label too long to keep says so.
- Ask your disk: a last-run date is matched by program name, so it now reads as "a program named 7zFM.exe last ran Nov 2026 (by name, any folder)" instead of passing for that copy's own date; last-access times are also read on a PC where Windows manages them itself ("System Managed, updates enabled").
- Search: `dc:` and `da:` say when the view has no times to read (no scan open, or a scan loaded from a CSV file) in the line under the box, a search too big to read says how many matches weren't checked instead of "Nothing found", and a redraw no longer drops what the reading limit said.
- Search: `dc:` and `da:` now count the matches read only in part in the status line instead of leaving them out of the list without a word, say "reading limit" only when the limit really stopped the reading (a skipped or unreadable part says so instead), call an empty folder empty instead of unreadable, no longer say a match `dc:` can't open is gone, and the line under the box follows a scan that finishes while the Search tab shows such a query.
- Folders the rules haven't explained: a label only takes a folder off that list when it names that one folder (an AI label with a wildcard no longer hides a whole tree), a labelled or hidden folder takes what is inside it off too, and an AI's labels and Forget now update the list on screen at once. The AI can label any folder by its full path, and a folder you hid no longer shows as its Owner or goes into the AI's prompts.
- Remembered: a rule the AI proposed keeps its "From the report only" note even when the AI's reason is long; the Hide, Forget and rule-file rows keep their buttons in view beside a long path.
- Search: hiding the column the list is sorted by sorts it by Name, so the sort always has a header to show it.
- Search: Ctrl+click, Shift+click or Ctrl+A as the first pick no longer makes Delete say the results changed, Delete with nothing picked no longer asks about a folder from a map tile you right-clicked in another view, and with several rows picked Open and Show in Explorer say "pick fewer" at once.
- Folders the rules haven't explained now stay explained once you know what they are: Hide takes a folder off that list (Remembered keeps the label, where Forget brings it back), and the AI marks every folder it looked at so the same ones aren't sent to it again and again.
- Search: sorting by Name now puts every folder above every file, in both directions (folders A-Z then files A-Z, or both reversed), for the scan on screen, other drives and saved scans.
- Search: when other drives have more matches than the list holds, the status says "showing the first N of M on other drives", and the list is the first N of all matches in the order you sorted by.
- Search: results you picked that a new sort pushes off the list are no longer picked, so Delete can't act on a row you can't see; emptying the search box drops the picks too.
- Search: after Ctrl+click takes a result out of the picks, Del and the row menu no longer still count it.
- Search: F2 renames the picked result (one at a time). The new name is checked first: nothing Windows doesn't allow, never onto a name that is already there, and a file keeps its extension unless you type a new one. Protected places and results from saved scans are refused, and Windows' own message is shown if it says no.
- Search: Ctrl+C copies the picked results as files, so Paste in Explorer copies them.
- Search: F2 rename is stricter. It is refused on network folders and under junctions, for a folder from another drive that holds a protected item (disk image, mail profile, vault) or has changed since the drive was read, with a filter on when the whole folder holds one, and for anything you told Sprawl to Keep or that holds it (a file inside a kept folder can still be renamed). A renamed row in the scan keeps its place and size, and a name like "Report v1.2 final" is no longer cut at its last dot. Typing in the rename box now follows where the cursor really is: Tab goes box, Rename, Cancel, and Space only types a space in the box.
- Search: F2 on a big folder from another drive no longer freezes Sprawl while it checks what the folder holds (a million items took many seconds, twice); it takes a fraction of a second now.
- Search: F2 renames items whose path is 260 characters or longer, which Windows used to answer with "cannot find the path".
- Search: Enter in the rename box without changing the name no longer renames a name that started or ended with a space; a case-only rename of an other-drive result says the list may still show the old spelling; a name like #FF102030 shows in the box as it is.
- Windows menu: it opens again at once after a command run from it (it was refused for about ten seconds); the folder rereads of every command are kept.
- Rename box: coming back to the window puts the focus where you left it (the box or a button), so typing isn't swallowed and Space can't press Rename by accident.
- Search: a failed Ctrl+C no longer wipes the clipboard or leaks memory, and the Windows menu waits while a reread after a Search rename is still running.
- Search: right-click the list's header to choose which columns to show (Size, Allocated, Modified, Folder; Name always stays). Your choice is remembered.
- Search: Delete (Del, Shift+Del, the Search bar's button and the row menu) acts only on results that are listed and picked. Something left picked under an earlier list, a folder picked in Tree View, or the folder a result moved up to after a rescan is no longer asked about; you are told to pick again.
- Search: with the map shown, Open, Show in Explorer and Copy Path work on the map tile you right-clicked even when it is not a result.
- Search: Show in Explorer on a result that is gone opens its nearest folder and says so, and a huge pick is refused at once instead of freezing the window while every path is read.
- Search: the Menu key on a row you just took out of the picks no longer drops the picks; the menu acts on them as Del does.
- Search: `attrib:h`, `attrib:s`, `attrib:r` and `attrib:l` now work on the scan on screen as they did on other drives, so `!attrib:h` no longer lists hidden files.
- Search: `childcount:0` and `childfilecount:0` list empty folders, not every file.
- Search: `dupe:` and `sizedupe:` say so instead of quietly changing the whole search when written as `!dupe:`, inside `<...>`, in `child:` or in `a|b`.
- Search: a half-typed size like `<10mb >` stays a size instead of becoming a group, and a deeply nested query or a pasted address list no longer freezes the window.
- Search: `exact:` and `case:` skip `dc:`/`da:` words (they stopped being dates), a quoted `"audio:"` is plain text, and `ext:jpg|audio:` keeps both lists.
- Search: the type examples in the help and the empty state are on two lines, so none is cut off.
- Search: "Use as filter" says it can't take a group instead of sending half of `<report 2024>|draft` to the filter box.

## 1.0.8 — 2026-11-23
- Search can find what was created, not only what was changed: `dc:` takes the same dates as `dm:` (`dc:2026`, `dc:2026-01-15`, `dc:>2026-01-01`, `dc:2026-01..2026-03`, `dc:lastweek`). A scan doesn't keep creation times, so they are read for the matches the rest of the search leaves; that needs another word to narrow it, only local fixed disks are read, and what the reading limit left out is said in the status line instead of being passed off as "no match".
- Search's `da:` (last opened) now works in the Search box as it already did for the AI: the same reading, the same limits, and the same plain words about what couldn't be read. It is checked for the scan on screen; other drives are searched without it and the status line says so.
- Search can look at the shape of folders: `parent:C:\Games` finds only what is directly inside that folder (`parent:steam` for a folder of that name), `child:*.tmp` finds folders that directly hold such a file or folder, and `childcount:>10000` and `childfilecount:>10000` count the files and folders directly inside one.
- Search can list duplicates: `dupe:` keeps only the files and folders whose name another match has (any case), and `sizedupe:` only the files whose size another file match has.
- Search can ask about attributes and long paths: `attrib:h` hidden, `attrib:s` system, `attrib:r` read-only, `attrib:l` a link or junction, `attrib:c` compressed, together as `attrib:hs`, and `len:>260` for a full path longer than 260 characters (also `len:<=200`, `len:100..260`).
- Search has type macros: `audio:`, `video:`, `pic:`, `doc:`, `exe:` and `zip:` mean the same extension lists the Type filter offers.
- Search's `<>` groups words: `<report 2024>|draft` finds both words of the first group or the second, groups nest, and `!<temp cache>` leaves out anything that has both. Quotes keep `<` and `>` as plain text, and a `<` with no closing `>` is still an ordinary character, so `>1gb`, `size:<1mb`, `dm:>2026-01-15` and `len:>260` mean what they did.
- The Search box's keyword suggestions and its help list know every new keyword, and the filter buttons show them (`parent:` under Where; `child:`, `dupe:`, `sizedupe:`, `attrib:` and `len:` under More).

## 1.0.8 - 2026-11-23

- Search results can be right-clicked without losing the rows you picked, and the menu now offers Open, Show in Explorer, Copy Path, Delete and Delete Permanently on every picked result (the items that only work on the scan on screen are shown greyed on results from other drives or saved scans). Open and Show in Explorer work on up to 10 picked results or folders at a time and say so rather than opening a burst of windows. Pick several with Ctrl+click, Shift+click or Ctrl+A, then Del or Shift+Del deletes them all.
- Ask your disk now says when a file or folder was last opened, and when a program last ran, so the AI can tell which of several copies was used last instead of saying Sprawl doesn't record it. Where Windows has last-access times turned off, the drive isn't a local fixed disk, or the answer's reading limit stopped it, the answer says so instead of passing it off as "not opened".
- Remembered: a proposed rule now shows its whole explanation, wrapped over as many lines as it needs, instead of being cut off mid-word after the first line; long names and paths that still have to fit one line show the whole text when you hover over them.
- Faster: the map is ready sooner after Sprawl starts (its graphics are prepared while the window opens), clicking a column header in Tree View sorts a big scan in a few milliseconds instead of a fifth of a second, clicking the same column again in File View just turns the list around, and a map at rest no longer redraws its tiles in the background, so a large scan no longer slows down every click and key press.
- The one-time reset of the theme settings that came with 1.0.7 is gone: your choices stay as you set them.
- The Search box takes the keyboard focus when the Search tab opens, also when Sprawl starts on it, so you can type at once.
- Cleaners, Unknown applications: one button asks the AI about all the folders in one go, instead of a button on each row.
- Settings takes the keyboard focus when it opens, and gives it back when it closes.
- Scan history takes much less space: each scan saves only what changed since the one before.
- Scanning a folder again is not saved as its own history point when a scan of the drive above it, made in the last day, already shows it.
- Cleaners, Privacy: with "Show privacy cleaners" on, Sprawl can now clear the short lists Explorer keeps in your own account's registry: the Run box, paths typed in the address bar, the Explorer search box, the Open and Save dialogs' recent files and folders, and recent documents. Each list is one part you tick yourself, and the review shows how many entries it would remove (they free no disk space). Every value is copied to a backup in Sprawl's data folder before anything goes, the keys themselves stay, and after the clean-up "Put back" writes the removed values back (values written since stay). The backups are also listed in Settings > Cleaning, and the last ten are kept. Explorer keeps some of these lists in memory, so a change may only show after you sign out and back in.
- Right-click a file or folder in the tree and pick "Overwrite and delete..." to write over its files once with random data before deleting them, so what was on the disk is harder to recover. Nothing goes to the Recycle Bin. Several rows can be picked at once. A file's named streams are written over too (and a folder's own), and a sparse file is only written where its data is, so its holes are not filled in. Before anything runs, the question says what it will do and what it cannot promise for that item: a solid-state drive (or one Sprawl can't tell about) moves writes around, compressed, sparse, encrypted or Windows-compressed files and copy-on-write volumes may keep the old data elsewhere, an overwritten NTFS-compressed or Windows-compressed (WOF) file takes its full length on the disk until it goes and the compressed bytes already there may not be overwritten, a OneDrive folder may keep older versions online, old names can stay in the file system's own records, and System Restore, shadow copies and the Recycle Bin may hold older copies. Files in use, changed since the scan, kept, protected, links, cloud files and files with another name (a hard link) are left alone, a read-only mark is put back when an attempt can't start, and Stop ends it after the file it is on. A file that is damaged by a failed attempt is named first. It never claims the data is gone for good.
- File > Wipe Free Space on the Selected Drive... runs Windows' own cipher over the unused room on the drive, so deleted files there are harder to recover. The question says it can take hours and that the drive fills up while it runs, and states the limits for that drive (solid-state or unknown, ReFS, shadow copies). Stop ends it and clears cipher's working folder if one is left; a folder an earlier Sprawl wipe left behind is noted and removed before starting, and another Sprawl wiping the same drive at the same time is refused.
- Privacy cleaning can now take things out of a browser's own database, with the browser closed: Firefox's browsing history (bookmarks stay, with their keywords and notes), Firefox's form history, and the autofill entries of Chrome, Edge and Brave (saved addresses, cards, passwords and search engines stay). Before anything is changed, Sprawl copies the database and its journal files into `%LOCALAPPDATA%\Sprawl\backups\privacy` and checks the copy; the last five copies of each part are kept and copies older than 30 days are dropped. The review says how many rows would go, never a size, and a database may not get smaller after an edit.
- The review and its result now say "row" for these parts, and the wording about privacy files no longer says the browser starts new, empty files when a database was edited instead.

## 1.0.7 — 2026-11-23

- The theme goes back to the defaults once, so everyone gets the new default look (below) and "Like Windows" for light or dark. Your saved Custom picks on the Theme page stay, ready to pick again.
- After a clean-up, reading the folder again to show what is left is much quicker: it no longer opens every file inside (minutes on a Temp folder of 600,000 files). The row says "Measuring what's left..." meanwhile, and Stop ends it.
- The biggest bar in the scan's file-type race has its rounded end again: it was a few pixels wider than its column, which cut it off square.
- A new default look: surfaces with a touch of your accent, slightly rounder corners (6 / 10), quieter 12 px explanations, rectangular chips and a flat finish. A look you picked on the Theme page stays as it is; the Midnight and Paper presets are unchanged.
- Menus open smoothly: File, Options and Help, every right-click menu and the dropdowns (More views, Colour, Search in, saved searches) fade in from just above with a soft shadow, instead of sliding and unrolling. Moving the pointer to another title while a menu is open switches to it, the keyboard works as before (Alt, Alt+F/O/H, the arrows, Esc), and the focus goes back where it was when a menu closes.
- The menu key and Shift+F10 open a row's menu in the tree, at the row.
- A treemap tile's menu acts on that tile only: Delete from it no longer includes rows picked earlier in the tree, or a result picked from another drive in Search.
- Tick boxes draw their tick in, and switches slide their knob with a small shadow.
- The shadow and glow round buttons are smooth at their rounded corners at every display scale (they could show a jagged ring).
- The Settings window and the yes/no questions fit a short window: their content scrolls and the buttons stay in view. The main window can't be made smaller than 1000 x 600, so nothing in it is cut off.
- The first time Sprawl opens, its window is a normal size in the middle of the screen, not most of a wide screen.
- The 3D map: its dark background and floor no longer show bands, and the map always fills its panel after the window is resized or maximised (it could stay at its old width).
- Show in Explorer selects the file again when its path has spaces in it.
- More choices on the Theme page: the window backdrop (solid, Mica or Acrylic), a stronger accent, colour-blind safe risk colours (blue, orange, vermillion and purple, with a different shape for each risk so colour is never the only sign; the words and what is ticked do not change), chips as pills, soft pills or rectangles, page tabs as an underline or as pills, text boxes and dropdowns outlined, soft or as Windows draws them, the selected row as a tint, an accent bar or both, how buttons answer the pointer (lift and sink, press-in, a plain fade, or a faint wash), and a flat finish with no shadows.
- Smoother look: real soft shadows (drawn by Windows' compositor) under the cards, the buttons and the Advisor's rows, following their rounded corners; the main button's is in your accent colour, and under the pointer a button's or row's shadow grows and while pressed it goes. Buttons have a lit top edge, a pressed button darkens (the pointer lightens it) with each press style's own timing, and the flat finish drops every shadow and lit edge.
- Softer finish: text boxes and dropdowns you are typing in (or have picked) get a soft ring of your accent colour round them, the cards and panes and the Settings window sit on a soft shadow, the main button (Scan, Save, Next) has a soft shadow in your accent colour that grows when you point at it, and the line under the open tab slides to the tab you pick (unless Windows' animation effects are off, or the look's motion is None).
- Search: results from other drives and saved scans can be picked (Ctrl+click, Shift+click, Ctrl+A) and deleted, to the Recycle Bin with Del or for good with Shift+Del. Before anything goes, each item is checked on the disk as it is now: still there, still a file or a folder, not a link, not protected, and a file still the size and date the result showed (a changed one is left alone and listed). A folder is measured first, so the question shows what it holds now next to what the saved read said. Where a drive has no Recycle Bin, the question says so and only offers deleting for good. Results from the scan on screen and from other drives can't be picked together.
- Settings has a Theme page: a sample that follows each pick, presets (Sprawl, Windows, Midnight, Paper, Custom), and the accent, surfaces, contrast, folder icons, corners, tick boxes, bars, motion and small text, with Copy theme and Paste theme to share a look. Light or dark moved there from General. Each pick shows at once on the whole window, Settings staying open; Save keeps it and Cancel goes back. Menus, lists and tooltips take a touch of the look, and every accent-coloured control (main buttons, chips, tabs, bars) takes the accent picked.
- Ticking or opening a row in the Advisor and Cleaners changes only that part of the panel: the row keeps its highlight instead of flashing again when the mouse moves, the panel no longer blinks, and clicks answer faster. An opened Advisor finding or Cleaners app row no longer flashes bright grey when the mouse leaves it.

- Buttons follow Windows 11: the standard size (14 px text, 32 px high, 4 px corners), the one main action of each place in your Windows accent colour, and a red button for deleting for good, so it no longer looks like ticking the safe ones. Selected chips and map modes are filled with the accent; text actions are plain until you point at them.
- Update and restart from a Sprawl running as administrator works with one click: the copy that reopens without administrator rights waits for the administrator one to close, instead of stopping with "could not be closed" until you click again.
- After Setup (an update, or Install for all users), Sprawl opens without administrator rights unless Always Run as Administrator is on, and the older per-user copy is removed even when Sprawl opens as administrator.
- The older per-user copy, started from its old shortcut after installing for all users, opens the copy in Program Files instead of offering to install again.
- Review selection opens again when the ticked items mix ones deleted for good with a program's own clean-up (or the Recycle Bin): the review's first line quotes "Deleted for good", and that quote stopped the whole review from being drawn, so the button seemed to do nothing. If part of the Advisor ever can't be drawn, Sprawl now says so.
- Settings > General > Theme is saved now (it always went back to Like Windows). A change shows when Sprawl starts next.
- An application's tick in Cleaners and in the Advisor ticks what its box shows, your own files included (the review asks for "my own files"): a cleaner for a project in Documents showed a box that ticked nothing.
- Emptying the Recycle Bin is held, with its reason, when Sprawl runs as a different account from the one signed in: it would empty that account's bin, not yours.
- Rows picked with Ctrl or Shift are dropped when a clean-up's rescan no longer finds them. They stayed behind counting nothing, and a later Delete could still catch them.
- A folder scanned again after a clean-up takes its new identity, so it can be cleaned or compressed straight away instead of only after a whole new scan.
- Replacing duplicates with links leaves a copy alone when its owner, permissions or read-only, hidden, system, compressed or encrypted marks differ from the file it would be linked to: one file can only have one set of each, so the copy would quietly take the other's. Because of that, this often does nothing for copies in different folders, and the suggestion says so.
- Closing Sprawl during a link run waits for the instant a copy has no name of its own, so no file is left under its backup name. The duplicate search now lists any file an earlier interrupted run did leave under a ".sprawl-original" name, and says how to tell whether to rename it back or delete it.
- The fast scan (as administrator) checks the drive's file table more strictly and reads folders instead when something doesn't add up: a boot record that isn't plain, a file table larger than the volume, pieces with a gap or an overlap, or a record that belongs to another file.
- Sparse and NTFS-compressed files show what they really take on disk in an ordinary scan, instead of their size rounded up to a cluster.
- If reading a folder goes wrong in a way Sprawl didn't expect, the scan counts that folder as unreadable and finishes, instead of waiting for ever.
- A pattern with an invisible character in it (a tab, a zero-width space, a mark that reverses text) is refused, so the approval card shows exactly what will be checked.
- A new or renamed custom AI can't take another's name, and a label the AI wrote says so in the Owner column.
- A check owed after a rule was approved in Remembered starts as soon as the running check ends, not when you next open Remembered.
- Each Claude Code chat runs in a folder made for it and removed when it ends, and an answer over 16 MB is not taken: Sprawl says so instead of working from half of it.
- An AI key belongs to one server, host and port both, so a key saved for a local server on one port isn't sent to another server on the same PC. Keys saved before this are moved to the server they were saved for, once.
- "Allow for this session" explains itself: the AI searches without asking until Sprawl closes, each search shows on the status line, and a search can carry folder and project names from your scan (your name and the PC's are taken out).
- Privacy > History (Chrome, Edge, Brave) says what stays: the downloaded files, and some traces — site icons, address-bar predictions and the last session. Two profiles with the same name are told apart by their folder in the title.
- A scan an MCP client asks for is saved to History as folders only, so it doesn't push out your own snapshots; a client that connects and sends nothing is disconnected instead of holding the pipe against the others; and a window where the bridge couldn't start writes the reason to bridge.log.
- Restart as administrator from a scan of several folders opens all of them again, instead of trying to scan the word "Multiple", and says how many ticks didn't fit on the command line so you can tick those again.
- Uninstall removes the recovery copy an earlier update left, so no old Sprawl stays runnable in Program Files.
- Light or dark is read straight from Windows' setting; Sprawl no longer starts a program to ask.

## 1.0.5 — 2026-11-21

- Codex, and Claude Code without Sprawl's server, can propose recognisers and cleaners now. The AI writes its proposals at the end of its answer, and Sprawl checks each one the way it checks any AI proposal. Each one waits in Advisor > Remembered for you to approve; ones Sprawl refused are listed under the answer.
- When the AI proposes something, a strip under its answer says so. Its "Review and approve" button takes you to Remembered.
- Approving or undoing a rule no longer checks the findings again each time. They are checked once when you leave Remembered, and still are if Sprawl was closed or crashed first.
- AI setup:
  - a dropdown beside Address fills in a well-known service (OpenAI, DeepSeek, OpenRouter, Gemini, xAI, Mistral, Groq, Together, Ollama, LM Studio);
  - Load models sits beside Model;
  - Load models uses a key you typed but haven't saved yet (it said the key was refused);
  - the rows fit a narrow AI column.
- A finding's details show the last time a file inside it was written, so you can tell whether a folder is still in use.
- Stained Glass no longer slides sideways without end: it settles and stays still.
- Clean-up:
  - files meant for the Recycle Bin on a network or removable drive, which has no bin, are held with the reason, not left hanging;
  - after Stop, an item Windows can't stop part way (a big file being compressed) says it is finishing; the run ends when it does, so what it freed and compressed is still recorded;
  - closing Sprawl during a clean-up stops it after the item in hand, then closes. Close again to close at once;
  - emptying the Recycle Bin runs first when the same run sends files to the bin, so it never erases them;
  - a virtual disk shrink (WSL, Docker) that is stopped detaches the disk again, so WSL and Docker can start;
  - "Tick enough for N GB" never picks the Recycle Bin, Windows.old, git worktrees or old clones; you tick those yourself;
  - approving a rule holds Review and Clean up until the findings are checked again with it.
- The AI card's header shortens a long model name instead of cutting off Change.
- Search no longer moves up a few pixels when you switch to it.
- The start page (Tree and File view before a scan) has the same background as every other page.
- Light mode: Settings > General > Theme picks Light, Dark, or Like Windows. The map keeps its dark look.
- The page tabs are plain text, with a line under the one that is open. Advisor and Cleaners show how many findings and cleaners have something for you.
- Several custom AIs: name an OpenAI-compatible server in AI setup and it stays in the Provider list. Each server keeps its own key, sent only to that server. A server that needs no key works with the Key box left empty.
- Sprawl installs for all users, in Program Files. It runs as administrator only from there: a copy in your own folder could be changed by any program running as you, which would then get administrator rights too. A copy in Program Files updates by downloading the new Setup, checking it against the signed release, and running it (Windows asks for administrator rights). The first start after installing removes the older per-user copy; your settings and scans stay. From a per-user copy, Restart as Administrator offers to install for all users.
- Dev folders:
  - Sprawl reads each repository's git settings itself first, and leaves a repository alone when they name a program, a filter, a hook or another place. Git is never asked about scanned repositories while Sprawl runs as administrator;
  - an old clone counts as your own files: it goes to the Recycle Bin, needs "my own files" ticked, and is offered only when its remote is a server, no commit (a detached checkout or a tag included) is missing from it, and no file is hidden from git status.
- The Advisor's memory: when the file can't be read (a typo, or saved as UTF-16), Sprawl says so, writes nothing over it and holds clean-ups, instead of starting with an empty Keep list. Two Sprawl windows no longer erase each other's Keeps.
- AI:
  - a key you typed but didn't save is forgotten when you pick another provider or service, so it is never sent to the wrong one;
  - Forget key deletes a saved key; removing a custom AI deletes its server's key unless another AI on the list uses it;
  - README and manifest text the AI reads is marked as data, and notes an AI answer left are kept apart from your own preferences;
  - a cleaner the AI proposes for folders outside app data counts as your own files;
  - a proposal with an absurd size no longer stops the memory from saving.
- Updating asks before it closes other Sprawl windows (or an AI app connected to Sprawl), and an old recovery folder no longer blocks updates.
- The fast scan (as administrator) counts what WOF-compressed files take on disk, and reports damaged file records instead of leaving them, and what is in them, out without a word.
- Colours and Tweaks is a Tweaks button in the map bar now, shown while the Treemap is on (it does nothing in the other views); Ctrl+T still opens it.

## 1.0.4 — 2026-11-20

- Scanning a well-used system drive as administrator is much faster: when the drive's file table had become fragmented enough that Windows keeps its list of pieces apart, Sprawl couldn't use its fast read and went through every folder instead. It reads that list now: on a test C: drive, 7.6 seconds from Scan to result instead of 17.
- A folder compressed partly (some files were busy) stops being offered once a retry goes through without problems, even when that retry had nothing more to compress: the busy files turned out not to get smaller, or were gone.
- Dev folders: the Advisor finds old git worktrees (removed with git's own "git worktree remove", which refuses one with changes), git history that "git gc" could pack smaller, clones untouched for half a year with everything on their remote (they can be downloaded again), and the same repository cloned twice. It asks git, read only; anything uncommitted, unpushed or stashed is never suggested, and Sprawl never deletes inside .git. When Sprawl runs as administrator, git is only asked if you allowed tools from your own folders in Settings.
- Windows' own clean-ups run inside Sprawl: tick the component store (DISM), the Recycle Bin or Windows.old (Disk Clean-up) and review them like anything else. Sprawl runs the command and shows what it freed; when one needs administrator rights, the bar offers "Restart as administrator". They were "Another account's" and couldn't be ticked.
- Compressing: pick the compression for each folder (XPRESS4K, 8K, 16K or LZX) in its details; the estimate follows the choice.
- The selection bar keeps the count, Review selection and the tick buttons together on one line, and says what the ticked items take on disk now and about what is left after.
- Deleting from the tree shows a card with a progress bar, how much is gone, and Stop. The tree can be used meanwhile.
- The Cleaners tab keeps its list while a drive is scanned, and says that it looks at this account's and Windows' own folders whatever drive is scanned.
- Clicking an orange or grey item's box for the first time no longer moves the box (the note under the item made the row taller, and the box re-centred).
- The AI chat no longer tells Codex and other AIs to "Use Ask Claude": it names the Ask button.
- After a clean-up, a folder scanned again is checked against the drive it is on now, so files on a drive that was reassigned since the first scan are no longer held back as "changed".
- AI setup has an Effort setting for each model: how hard the model thinks before it answers. It lists only the levels that model offers: read from Anthropic's model list and from Codex's own model list, or taken from the providers' documentation otherwise. A model without effort levels, or one Sprawl doesn't know, shows no Effort setting, and its provider's own default applies.
- AI setup picks the provider from a dropdown, like the model, instead of a row of buttons that didn't fit: "Off" was cut off. With no Anthropic key saved yet, the Model dropdown lists the usual Claude models.

## 1.0.3 — 2026-11-19

- Compressing a folder is much faster: Sprawl compresses the files itself, several at a time, instead of running compact.exe on them in batches. While it runs, the item and the header show how far through it is ("Compressing... 40%, 1.2 GiB of 3.2 GiB"); Stop ends it after the files being compressed at that moment.
- Each compression setting (XPRESS4K, 8K, 16K, LZX) is stored as chosen. A file that changed or was replaced after Sprawl planned the run is left alone. A file Windows won't let Sprawl read counts as a failure, with its reason, not as "in use". The saving counts every data stream of a file both before and after.
- The clean-up log says what a compress did (compressed, already compressed, in use, changed, failed) instead of naming compact.exe.
- After compressing, the banner says the files were compressed in place and nothing was deleted (it said "deleted for good" before), how many files were compressed and what was saved, and how many a program had open and were left as they were.
- A folder Sprawl has compressed completely is no longer offered for compressing again (Windows doesn't show compression in a folder listing), until it grows by a tenth, after a big update say. One stopped part way, or with files a program had open, is still offered so you can finish it; either can be undone under Compressed, and compressing again keeps the earlier saving.
- A compress suggestion's button says "Compress this...", and its text says nothing is deleted.
- "Clean up just this..." on one item leaves the other ticked items ticked, also while the Advisor reads the folder again afterwards; a stopped clean-up leaves the items it didn't start ticked.
- A question pasted into the AI chat with line breaks shows them as line breaks, not as "�".
- Linking duplicates: the original is made read-only before its last check, so no new data stream can be added to it after that check. A copy that has another name (a hard link elsewhere) is left alone, since linking it would free nothing and would leave that other name read-only.
- A clean-up treats an item as the one scanned only when it has the same full file id on the same drive, the drive read when it was scanned (each folder of a multi-folder scan on its own). An item on another drive now, after a drive letter was given to another disk, stays. In a folder that changed since the scan, an item without an id stays. The preview of what would go uses the same test.
- npm's cache clean-up holds the cache and its logs folder from before npm is first asked where its cache is.
- Codex: a turn that finishes without an answer of its own is reported as such, never answered with the previous turn's text.
- An app's last use is taken from its own folder only, never from a folder whose name differs only in case, also when both were scanned as separate folders. A recent item still matches when only the drive letter's case differs.
- Sprawl's own temporary folders are removed only when they are provably the same folder it made.
- The scan's Details line about network folders gives the number on the network, on this PC and unknown, once each.

## 1.0.2 — 2026-11-19

- Saved clean-up scripts run only Sprawl's own deletes: a program's own clean-up and compressing a folder are listed to do by hand.
- Compressing a folder is refused when it is in, or holds, a folder you skip, before anything in it is read; Stop also ends the check.
- When Sprawl runs as administrator, an MCP client that doesn't gets a scan report without the "folders the rules haven't explained" part, and apps and recent items only for folders on screen with that exact name.
- Shares whose name begins with a dot (\\nas\.photos) can be scanned again.
- A folder skipped on a drive letter that isn't there now (a disconnected network drive) is also skipped when a share is scanned by its own name.
- Whether this PC serves a share is asked of Windows' Server service, so a temporary share is known too. The folder it serves is taken only from the service's own answer, which needs administrator rights: if a personal folder (Documents, say) is redirected to a share this PC serves and Sprawl isn't running as administrator, clean-ups are held, since that folder can't be confirmed.
- A clean-up is held when the Keep list's file is a link that leads nowhere, instead of counting nothing as kept; a Delete keeps every Keep it saw while it ran.
- The AI request log is written, rotated and cleared through checked handles, also as administrator; clearing the search history does the same.
- AI answers show paths as written, including brackets, stars and tildes in their names (C:\[draft](old).txt).
- A file name whose letters change width when lowercased (rare symbols) no longer crashes Sprawl when Search shows it.
- After a command from the Windows menu, Delete works again once Sprawl has reread that folder, instead of waiting until a restart.
- The review's button always names what is deleted for good, however many kinds of item are ticked; compression and shrinking a virtual disk get their own groups.
- Compressing a folder is refused when it holds a link (junction) that compact.exe could follow out of it.
- When Sprawl runs as administrator, an MCP client that doesn't (such as Claude Desktop) is answered only from the scan on screen: it can't start new scans or read files through Sprawl. A switch in Settings > Scanning allows it.
- When Windows can't say whether Sprawl runs as administrator, Claude Code, Codex and the Windows menu treat it as administrator (they need the switch in Settings).
- Codex's answer is read even if a helper it started keeps running; rules with many * in a name pattern can no longer stall a clean-up.
- The header says what was scanned and on which drive, on two lines; a Details link beside it explains what the scan covered: how it was read, what couldn't be read, the folders skipped by your settings, and the drives it didn't include.
- Advisor: with AI off, one line replaces the AI column; with nothing ticked, the bottom bar is one row; Clean up is now "Review selection...". Cleaners shows its state once, in the tab.
- The review before a clean-up groups what is ticked by what will happen to it (deleted for good first, then the Recycle Bin, programs' own clean-ups, links, and what is held back), and its button says what it does, such as "Delete 1 for good, recycle 2".
- Search: one search box (the Filter box and the file types pane step aside), a "Search in" menu for other drives, the map only when you press Map, and a message saying what was searched when nothing matches.
- The first start shows the welcome page across the whole window, not under two empty panes.
- The tree's top row starts with the scanned folder's own name, and long folder paths in File view and Search keep their drive and last folders (the whole path is in the tooltip).
- Map labels that would overlap are no longer drawn on top of each other, and the map's animations follow Windows' Animation effects setting until you set Sprawl's own.
- AI answers draw folder trees, tables and code in a fixed-width font with their lines intact, keep the answer's line breaks, and show nested lists, quotes, links (with their address) and struck-through text.
- Codex can be the Advisor's AI, like Claude Code: it uses your own Codex sign-in (no key), found on PATH or in the Codex app. It answers from the scan's report. Before each answer Sprawl checks, without sending anything, that Codex would start no MCP server and that its tool features are off, and doesn't start a Codex set up otherwise. Codex can't be started without tools, though: it still offers its model a code runner and helper agents. Those may still run, and Sprawl can't verify what they will do; AI setup says so when you choose Codex, and choosing it accepts that. The other AI choices get only Sprawl's own read-only tools. AI setup no longer suggests Codex on its own.
- The Advisor no longer lists the scanned folder itself under "Folders the rules haven't explained".
- History keeps to its size limit (Settings) when many folders have full snapshots: older full snapshots are removed too, never a folder's newest.
- Safer updating as administrator: Sprawl reopens itself without administrator rights through Windows' own desktop and writes no file while elevated. Sprawl no longer deletes the low-space task made before 1.0 ("Sprawl low-space check"): if you have it, it follows the alert setting, and you can remove it in Task Scheduler.
- Moving a folder to the Recycle Bin whole now checks every file as carefully as one at a time (a file moved in after the scan stays), and Stop is obeyed before the move.
- Nothing is ever run on a network folder, including commands such as compressing a game library on a share; the Windows menu is not offered for network rows.
- Delete in the tree checks the folders to skip and your Keep choices again before each item. It waits for Check again, and closing Sprawl during a Delete stops it after the current item.
- While scanning with skipped folders, nothing of them shows (names or file types).
- Folder scans read each folder through a checked handle, so a folder swapped for a link mid-scan is never listed. They run at the same speed.
- If the list of folders to skip can't be read, network scans stop too. A folder skipped on a mapped drive is also skipped when the share is scanned by its own name.
- Documents (or another personal folder) on a share served by this PC keeps its protection.
- Compressed files are measured correctly after compressing; file times before 1970 compare correctly.
- Cleaners works without administrator rights again: a folder this account can't open (such as Windows\Temp) is left out and named with Windows' reason, instead of the whole check failing. If a later check fails, the earlier list and what was ticked in it go, so nothing out of date can be cleaned.
- A program's own clean-up (npm, pip, NuGet, Go) is told the folder you reviewed, asked first where it would clear (its whole answer, not just the last lines), and run only when that is the folder, which is held while it runs. npm keeps its own logs in that folder too, and Go ignores its saved settings and flags for it. Sprawl no longer runs the clean-ups of conda, pnpm and Yarn, which also clear places outside the folder reviewed: run `conda clean`, `pnpm store prune` or `yarn cache clean` yourself.
- An old folder moved in after the scan is left alone by a clean-up, like a new one. In a folder that changed since the scan, only the very files and folders the scan saw there are gone through (Sprawl knows each by its file ID); anything else, such as an old folder put in place of the one reviewed, stays. When the scan didn't record file IDs (a CSV file, History), a changed folder is left whole until it is scanned again.
- If your Keep choices can't be read, Delete holds the item and says so, instead of going ahead without them.
- Folders to skip on network drives still apply when a drive's mapping can't be read; Sprawl then treats every network place, and every drive letter not known to be on this PC, as possibly skipped.
- MCP: the client's own account and administrator rights are read from the pipe itself.
- With the Recycle Bin on, a link (junction or symbolic link) is left where it is with a note, instead of being deleted as a link.
- Privacy items are named in the review's button and heading as deleted without the Recycle Bin ("Delete 1 item and 1 privacy item without the Recycle Bin"), and counted as sets. The review's first line names each group ("What is under "Deleted for good""), never "the rest".
- The review's first sentence comes from the same list as its groups: compressing or shrinking a virtual disk no longer reads "Deleted for good".
- The Details card no longer claims every file was counted or every folder read when that isn't known, and says when the scanned folders were read in different ways or are partly on the network (as the scan found each one when it opened it, or that it couldn't tell). Finishing a scan no longer asks every drive for its name. Restarting as administrator "may" read more, not "will".
- Duplicate files are made links only when neither has an extra stream at all (such as a download mark). Before the original is removed, Windows is told to delete it, which stops any new stream being added; its streams are then checked once more, and if one appeared, the original is kept.
- AI answers show paths and wildcards as written (C:\Cache\*.log keeps its backslash), keep tildes and backticks in names, show a link's whole address, and show a table with more than 12 columns as written instead of dropping columns.

## 1.0.1 — 2026-11-19

- Scans are fast again on drives and folders read without the MFT (exFAT and FAT drives, the search index, the daily background scan). 1.0 re-checked every folder's whole path before listing it, which made these scans many times slower.
- Skipping folders (Settings > Scanning) no longer turns off the fast MFT scan: whole drives scan as fast as without skipped folders, which are left out as the tree is built.
- Network drives and `\\server\share` folders can be scanned again (read only; clean-up actions stay on local drives).
- Advice after a scan is quicker: last-use times come from folder listings instead of opening every file.
- Folders go to the Recycle Bin whole again when nothing in them must stay, as one entry Explorer can restore, instead of every file alone with the empty folders left behind.
- Updates work again with Options > Always Run as Administrator on: Sprawl reopens itself without administrator rights to install the update, then comes back as usual. 1.0 refused to update while elevated.
- Delete in the tree no longer freezes the window on a big folder: it runs in the background with progress in the footer.
- Recent scans on the welcome page list each folder once.
- Deleting works again on PCs where Documents, Desktop or another personal folder is redirected to a network share. 1.0 could not check those folders and refused every delete.
- Changing a setting no longer fails silently when Sprawl reads its skipped folders at the same moment.
- The assistant can read project manifests again when it works through Claude Code, and copies of extracted archives are found in scans opened from History.
- Turning alerts off now also removes the scheduled task made before 1.0, including when Sprawl always runs as administrator.
- If Always Run as Administrator is on, 1.0.0 cannot install this update itself: turn the option off once, update, then turn it back on, or run Setup.
- Known limitations and open questions: [docs/plans/status.md](docs/plans/status.md).

## 1.0.0 — 2026-11-19

### Replacement build: scan regressions

- Fix automatic drive selection rescanning the previously displayed root. Rapid source changes stop the old scanner and retain the latest request.
- Clear old Advisor results while scanning; prevent Advisor/Cleaner refreshes from analyzing the previous tree until the new scan settles. A failed or stopped scan explicitly identifies the retained previous source.
- Honor Stop even when the worker has already finished scanning and is sorting or waiting for the window to accept its result.
- Restore fast folder inventories by omitting expensive per-file hidden-WOF probes. Cleaner discovery reads its requested folders instead of choosing a whole-volume MFT read. Detailed storage-operation checks still verify WOF backing.
- Compression search uses metadata recorded by the scan: fast folder inventories may omit hidden WOF compression. MFT and detailed storage checks retain detection. This limitation avoids slowing every ordinary scan for an optional search attribute.

The 1.0.0 downloads and Setup have been rebuilt in place; this remains version 1.0.0.
Existing 1.0.0 installations need the replacement Setup or a manual executable replacement;
the normal version-based updater does not treat the same version as a newer release.

### Original 1.0.0 changes

- Recognise registered MSI packages, likely EXE installers and unregistered Steam library folders, with the evidence and limits shown.
- Compare cleanup, compression and guided Steam move options for a space goal, including destination capacity and costs.
- Show prevention advice after successful cleanup.
- Learn local cleanup and skip choices; stop automatically recommending fast-refilling caches and Steam libraries you choose to manage yourself. Learning can be reset separately from Keep decisions.
- Ask what a finding is and approve reusable AI recognisers for future scans.
- Settings now offers decimal sizes and exact folders to skip. Skipped scopes also prevent cleanup of a containing folder.
- Preserve folder-name space in narrow tree panes, and find filesystem-compressed items with `compressed:`. Tree labels distinguish filesystem compression from Sprawl's earlier compression records.
- Recheck cleanup targets, linked copies, Recycle Bin destinations, excluded scopes and protected data before acting. Undo compression uses the current Keep and exclusion choices.
- Preserve credential and personal-folder protections for relocated profiles and redirected Documents, Downloads and other Windows folders.
- Tie remembered cleaner choices and confirmation to the reviewed definitions, actions and scan. Late AI answers cannot apply suggestions to a different report.
- Keep MCP reports, facts and filters tied to their source scan, isolate connections by Windows account, and reject tools outside the advertised set.
- Keep History saves distinct during simultaneous scans, validate stored roots and bound snapshot decoding. Growth comparisons use observations earlier than the scan being discussed.
- Require known active/default versions before offering Rust or Node removals, and strengthen installer-source and extracted-ZIP evidence.
- Credit owner-tool savings only after success and complete comparable measurements. Failed or incomplete results do not teach successful cleanup.
- Keep a complete previous launch bundle for interrupted updates. Update and restart closes other Sprawl processes from the same account and installation, including lingering MCP servers.
- Detect WOF compression in detailed storage checks and MFT scans, and reconcile MFT top-folder progress after out-of-order parser results. See the replacement-build note for fast folder inventory limits.

The complete review dossier and corrections passed independent Astra review before release.

## 0.5.0 — 2026-11-18

### New
- **Compress programs and games in place.** The "Compress" tip can be ticked: Sprawl runs Windows' own compact.exe on the folder (XPRESS, the way Windows compresses itself), nothing is deleted and the program runs as before. Show > Compressed lists what you compressed, with Uncompress.
- **Shrink WSL and Docker disks.** Their disk files grow but never shrink. Tick one and Sprawl shuts WSL down (checking Docker Desktop is closed first), then compacts the file with Windows' diskpart, as administrator. Nothing inside is deleted; how much comes back is known only afterwards.
- **Older versions side by side**, each removed by its own tool, never by deleting its folder: Rust toolchains (rustup), Node.js versions (nvm for Windows, fnm), Android SDK build tools, platforms, NDKs and emulator images (sdkmanager). Older Python and .NET versions open Installed apps. Never the newest, the one in use, one a project in the scan asks for, or an image an emulator uses.
- **Notices when a drive is nearly full or a folder grows fast** (Settings > General, off by default). A small scheduled task checks every 4 hours and shows a Windows notice; a click opens Sprawl there. Growth compares your own scans; an optional daily light scan of your user folder keeps it current.
- **Tool commands show what they did**: the review shows the command, and the log keeps its output.
- Driver and installer leftovers in C:\NVIDIA, C:\AMD and C:\Autodesk are offered.

### Fixed
- A review where everything ticked is held back said "Clean up ?"; it now says nothing can run now.
- Installed apps whose entry names no folder (Python, .NET) were missing from what Sprawl knows about your apps.

## 0.4.1 — 2026-11-18

### New
- **Rows Sprawl doesn't tick can be ticked on a second click**: a tip, or a folder left behind that holds your files. The first click says why Sprawl doesn't tick it; the second ticks the whole folder, which then always goes to the Recycle Bin, and the review asks you to confirm. Sprawl's safety checks still apply. Windows, programs, OneDrive, your profile's own folders and anything risky get no box.
- **The AI can suggest findings to tick.** Its suggestion shows beside the list with a tick for each and "Tick all it suggested"; rows it ticked stay purple until you change them. Only safe and amber findings, checked again before they are ticked.
- **The AI sees what you ticked**: "Are my choices safe?" gets an answer for each item.
- **The AI box suggests questions from what is on screen**: what you ticked, the row you opened, what grew, folders left behind. More suggestions stay under each answer.

### Fixed
- The logo, name and version are back at the top right, and the About page has the logo.
- **Keyboard shortcuts** (Help) showed a broken character between each key and its action. The box is redone: grouped, in two columns, with each key shown as a key.
- A review no longer says "gone for good" for files that go to the Recycle Bin.

## 0.4.0 — 2026-11-18

### New
- **The AI sits beside your Findings**, as on the Cleaners tab: the list on the left, the AI on the right, each scrolling on its own. What it found is listed there with a tick each, and "Tick the safe ones it found". The separate Ask section is gone. "Hide the AI" folds the column away.
- **AI apps can scan for themselves** through Sprawl's tools (MCP): `scan_path` scans a folder in the background with your own rights, not Sprawl's administrator rights; `scan_status` and `list_scans` follow it. When Sprawl is open and not running as administrator, it does the scan. Network drives are refused. With History on, these scans are saved like your own.
- **Two more tools for AI apps**: `search`, with the Search tab's language, and `growth`, which compares a folder with what History saved some days ago. Findings now come with their rule and how sure it is.
- **Sprawl looks for an update every time it starts**, and once a day while it stays open.

### Fixed
- **Check again** found the same things after they were removed outside Sprawl (the Recycle Bin emptied, a command run from the review). It now reads what the findings point at again first.
- **History's Play** moved one step and stopped. It now plays to the end, and moving the timeline no longer freezes the window for a second: each scan is read in the background.
- "Play it in History" in the Advisor now shows the map and plays, where before nothing seemed to happen.
- Copies that took the first 0.3.2 get the Cleaners tab's scroll fix with this one.

## 0.3.2 — 2026-11-18

### Changed
- **Sprawl asks its questions in its own boxes**, in the window, in place of Windows' message boxes. Esc or No is the default; Yes needs a click, or the keyboard moved onto it first. If the scan changes while a delete question is open, nothing is deleted and Sprawl says why.
- **File View sorts in a fraction of a second** on millions of files. By folder it took up to a minute and a half on a full C: drive.
- Ticking or opening a row in the Advisor no longer rebuilds the whole list, so the list doesn't jump and the click is answered at once.

### Fixed
- **The Cleaners tab jumped back to the top** each time a row was opened or ticked further down the list. It now stays where it was, and so does the column beside it.
- Sprawl's tools for AI apps (MCP): the scan report lists the Advisor's findings again, and says when the rules haven't run yet. With Sprawl closed it answers from History's last saved scan and says how old that is. A tool call after Sprawl was restarted no longer fails.

## 0.3.1 — 2026-11-18

### Changed
- **The four "as administrator" switches are kept where only an administrator can change them** (the machine's registry, for your account). Before, they were in your settings file, which any program you run could edit to turn one on. They can now be changed only in Sprawl running as administrator; elsewhere they show greyed out. A switch you had turned on is off again until you turn it on from an elevated Sprawl.
- After "Tick the safe ones" or "Tick enough", the Advisor shows its Findings, where the ticks are, and each application in its Cleaners group says how many of its parts are ticked.

### Fixed
- The Advisor tab could show its header and nothing under it after a visit to the Cleaners tab.
- Community cleaners were downloaded when Sprawl opened on the Advisor or Cleaners tab even with the setting off.
- A folder name with letters such as ü or ö right after `%TEMP%` or `%APPDATA%` could stop the community download, or the check of a cleaner of your own, with a crash.
- Community cleaners can no longer name your own folders (Documents, Pictures, Downloads...), a BleachBit "file" action removes files only (never a whole folder), and a cleaner of your own can't run as administrator.
- Claude Code's chat: an answer to a web search from a stopped question no longer answers the next question's search.
- Web searches leave out paths written as addresses (`file:///C:/...`), and your user name or computer name inside a word.
- Claude Code's working folder is refused if it has been replaced by a link to somewhere else.
- The Windows menu checks the command chosen and gives add-ons more room for their command names.

## 0.3.0 — 2026-11-18

### New
- **Cleaners tab.** BleachBit-style: each application with its parts (caches, logs, crash dumps), what cleaning costs and whether the application is open. It reads only the folders the cleaners look at, so it works without a full scan. Your usual ticks can come back next time (a setting). Community cleaners from winapp2.ini and BleachBit can be turned on; they are off until you do.
- **Privacy cleaners, off by default** (Settings > Cleaning, "Show privacy cleaners"). Cookies and history of Chrome, Edge and Brave, and cookies of Firefox, one row per browser profile by its name. Each is ticked on its own, never for you, never counted as space, and has its own box to tick in the review. If the browser or another program holds one of its files, nothing in that set is removed and Sprawl says what to close. The files are deleted, not overwritten.
- **Search tab.** File names in the scan and, when you switch them on, your other drives (also without administrator rights). Filters under the box (Kind, Type, Size, Modified, Where), keywords such as `ext:`, `size:`, `dm:`, `path:` and `regex:`, suggestions as you type, saved and recent searches. Enter opens a result, Ctrl+Enter shows it in the tree.
- **Your own files go to the Recycle Bin** when the Advisor cleans up (Settings > Cleaning, on by default). Caches are still deleted, since in the bin they would free nothing. Sprawl counts what the bin already holds.
- **Windows' own right-click menu** on a row, with the add-ons from other programs.
- **The AI on the Cleaners tab**: ask it, and it can suggest parts to tick (safe and amber parts only; you still review). Claude Code's chat gets Sprawl's tools.
- **Work offline** (Settings > Network): one switch that stops the update check, community downloads and every AI request.
- **"Open Sprawl on"** (Settings > General): the tab you used last, or a tab you choose.
- One activity indicator beside the filter box for everything running in the background; click it for the list.

### Changed
- The Advisor finds more, and says how sure it is: folders left behind by removed applications (shown only when they may hold your work), programs built on Chromium, build outputs graded by how idle the project is, and growth taken from your scan history. Tips are ranked by what they gain, apart from deletes.
- **On a drive of games, media or backups** the Advisor now lists the biggest folders and files nothing has changed in for a year, as information: Sprawl doesn't remove them, you decide. Before, such a drive showed "nothing to clean".
- The thresholds the Advisor uses (how old, how idle, how big) are settings, each with a floor it can't go below.
- **Run as administrator:** the Windows menu, handing an app to your uninstaller, Claude Code and clean-up tools from your own folders stay off, as before, but each can now be allowed in Settings > Scanning, under one warning. A switch that is on is your answer: nothing asks again.
- A clean-up never touches a network path, anything new since the scan, or anything you chose to keep; the review holds back what won't run on this PC and says why. A saved script says what it doesn't protect.
- Nothing is read, fetched, chosen or ticked for a new user until they say so.
- Numbers keep their width while they count; tabs, chips and map buttons show pointer-over, pressed and keyboard focus the same way. The map folds away on the Advisor and Cleaners tabs, and on Search in a short window it takes less room.
- Times are shown in local time everywhere. Typing in the filter is quicker, and the tree's work no longer holds up the window.

### Fixed
- The AI on the Cleaners tab answers with nothing scanned (it uses what the tab read), and a part that can't be counted no longer holds the question for ever.
- A change to the AI or administrator settings takes effect when you press Save, not at the next start.
- A problem in the Advisor's analysis ends that check instead of the tab.
- Very long tables no longer stop drawing part-way.
- The result card no longer says files were "deleted for good" when nothing was removed.

## 0.2.5 — 2026-11-17

### Fixed
- **The setup wizard opens by itself the first time Sprawl starts.** Before, it waited for the first scan and then only showed a small "Set up Sprawl?" card, which the update card could cover.

## 0.2.4 — 2026-11-17

### Fixed
- The Model list now loads when you type or change the server address, not only when you switch provider. Before, an address typed afterwards (LM Studio on another PC, say) left you with a text box.
- Switching provider no longer keeps the old provider's model (such as a Claude model for an OpenAI-compatible server).

### Changed
- Claude Code gets a Model list too: the current Claude models (its command line can't list them). **Haiku is now its default**: fast, and the cheapest on your plan's limits.

## 0.2.3 — 2026-11-17

### New
- **The installer is now an Inno Setup installer**: the usual setup wizard, a choice of folder, an optional desktop shortcut, and Inno's own uninstaller. Copies installed with 0.2.0 to 0.2.2 move over when you run it.
- **Pick the AI model from a list.** In the AI settings and on the Settings page, Model is a dropdown filled from your provider (Anthropic with your key, or the OpenAI-compatible server's list, such as Ollama or LM Studio). If there is no list, you type the name as before, and Sprawl says why.

### Fixed
- The update card and the "Set up Sprawl?" card can be clicked and scrolled again (the welcome screen was covering them).

## 0.2.2 — 2026-11-17

Safer updates, from a safety review of the updater.

### Fixed
- Uninstalling now removes the program files even if you take a while to close the last message, or another Sprawl window is still open.
- Updates keep working if GitHub moves its download servers or the project is renamed.
- If the daily update check fails three times in a row, Sprawl says so instead of staying quiet; one offline start no longer skips the next day's check.
- A future release that adds a file no longer stops older copies from updating.
- Each downloaded file is checked again just before it is used.
- Updates can also be signed with a backup key kept offline.

## 0.2.1 — 2026-11-17

### Fixed
- After an update, the old copy closes its window normally instead of ending in a way Windows could report as a crash.

## 0.2.0 — 2026-11-17

The first release with an installer and automatic updates.

### New
- **Installer and updates.** `Sprawl-Setup-0.2.0.exe` installs Sprawl for you only, with no administrator rights, and adds it to the Start menu and to Settings > Apps. Sprawl looks for a new version once a day (Help > Check for Updates, and a switch to turn it off), shows what changed, and updates itself after you say so. Every update is signed and checked before it is used.
- **The Advisor tab.** Finds what can go: build outputs of idle projects, caches, old installers, temporary files and more, each with why it's safe or what it costs to get back. Clean up inside Sprawl: tick, review, and Sprawl removes it, with a log of what went.
- **Ask your disk.** An optional AI (Claude, OpenAI-compatible or a local model) explains unknown folders and answers questions about the scan. Sprawl can also run as an MCP server for Claude Code.
- **Growth over time.** History keeps your scans; the Growth tab shows what grew and when the drive will be full, and History plays it back as a time-lapse.
- **Seventeen more views** behind the More button, and one Colour switch (Types, Risk, Owner, Growth) for every view, with a key.
- **Multi-select** in the tree and the file list (Ctrl+click, Shift+click), with one confirmation for everything picked.
- One Settings page (Options > Settings, Ctrl+Shift+S) and a Help menu with the keyboard shortcuts.

### Changed
- The window fits your screen on first start and opens where you left it.
- Scan turns into Stop while scanning; F5 scans the folder on screen again.
- Copy Path, Export to CSV and Export File Types moved to Ctrl+Shift+P, E and X (Ctrl+Alt clashed with AltGr).
- Text is at least 12 px, the dimmest grey is brighter, and buttons and ticks have names for screen readers.
- The setup is offered in a small card after your first scan, instead of opening over it.

### Fixed
- A long list in the clean-up review no longer pushes its buttons out of reach.
- Sprawl's own deletes no longer show as changes from outside.
- A colour mode with nothing to show no longer darkens the whole map.

## 0.1.0 — 2026-11-16

The first version named Sprawl: a fast disk space analyser with a treemap, City and Galaxy views, MFT scanning as administrator, and a filter.
