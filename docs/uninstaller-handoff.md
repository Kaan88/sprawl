# Handing an uninstall to another app

Sprawl never runs an uninstall command itself. On an unused app's finding, **Open Installed
apps…** opens Windows' Installed apps (with the app's name copied to search for), and **Open
Steam's uninstall…** opens Steam's own.

An uninstaller app of your own can take over instead. To set one, open **Options > Settings**,
and under **Uninstaller** enter the full path to its `.exe`. From then on, the button reads
**Hand to <program>…** and starts that program, after you confirm. Sprawl passes the details below,
each as one `--key=value` argument:

```
<uninstaller.exe> --sprawl-uninstall --name=<display name> --kind=<kind>
                  [--publisher=<publisher>] [--steam-id=<id>] [--folder=<path>]...
```

Because each value is attached to its key with `=`, a value that starts with `-` can't be read
as a flag. When Sprawl itself runs as administrator, it doesn't start the uninstaller unless
**Options > Settings > Scanning > "As administrator, also allow: handing an application to your
uninstaller"** is on (off by default); then it asks once per session, because the uninstaller
will run as administrator too.

| Argument | Meaning |
|---|---|
| `--sprawl-uninstall` | Always first. It marks the call as coming from Sprawl. |
| `--name` | The app's display name: the uninstall entry's `DisplayName`, the package name, or the game's name. |
| `--kind` | One of `installed` (has an uninstall key), `store` (an AppX or MSIX package), `steam`, `epic`, `gog`, `wsl`. |
| `--publisher` | The publisher, when known. |
| `--steam-id` | The Steam app id, when known. |
| `--folder` | Can appear more than once. The install folder comes first, then the app's data folders. |

The values come from the scan and the registry, so the uninstaller should treat them as hints.
Before acting, it should look the app up itself, by name or folder, and ask the user to confirm.
Sprawl only starts the program. It doesn't wait for it, and it doesn't read its output. After an
uninstall, scan again to see what was freed.
