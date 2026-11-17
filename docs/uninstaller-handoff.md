# Handing an uninstall to another app

Sprawl never runs an uninstall command itself. By default, **Uninstall…** opens Windows'
Installed apps, or Steam's own uninstall.

An uninstaller app of your own can take over instead. To set one, open the Advisor's Ask
section and press **Change**. Under **Uninstaller**, enter the full path to its `.exe`. From
then on, **Uninstall…** starts that program. Sprawl passes the details below, and each value is
its own argument:

```
<uninstaller.exe> --sprawl-uninstall --name <display name> --kind <kind>
                  [--publisher <publisher>] [--steam-id <id>] [--folder <path>]...
```

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
