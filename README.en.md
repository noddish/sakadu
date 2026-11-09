[한국어](README.md) | **English**

# Sakadu

A Windows file manager that keeps several folders open side by side and finds any file on your disk as you type its name.

![Sakadu with the icon bar and favorites panel, a workspace, two panes, thumbnails and the preview pane](docs/screenshot-main.png)

**[Download the latest release](https://github.com/noddish/sakadu/releases/latest)** · Windows 11 · a single 1.2 MB exe, no installer · free and open source

> The interface is Korean only for now.

## Why you might like it

**Search without waiting.** Press Ctrl+F, type part of a name, and results from the whole disk show up right away.
On a PC with about 1.8 million files and folders, a search takes 10–20 ms.
Search stays open in the **search panel** on the left, so you keep working in the panes with the results beside you (Ctrl+Enter opens the location in the current pane).
While Sakadu is running, a global hotkey brings up search from any app. Turn on **트레이에서 대기** (stay in tray) and that works even after you close the window.
Add a shared folder or USB drive and it's searched too.
Paste a list of file names to find them all at once, then open or copy the ones you pick in one go.

![Results kept in the search panel and a photo pinned in the preview](docs/screenshot-search.png)

**Stop juggling windows.** Split the window into 2, 3 or 4 panes, each with its own tabs. Drag a tab to move it to another pane.
Moving files from Downloads into a project folder happens on one screen.

**Works like Explorer.** Right-click menus, the copy progress dialog and drag and drop behave exactly as they do in Explorer.
Moved something by mistake? Ctrl+Z.

**Look before you open.** Thumbnail view and a preview pane show photos, PDFs and text files right next to the list. **Pin** a reference file in the preview and it stays there while you select other files.

**Pick up where you left off.** Keep panes and tabs, the search query and the pinned preview file as **workspaces** and switch between them. A new tab opens on your favorite folders and recent locations.

**Light.** No runtime or web view. It's Rust drawing with Direct2D. Unzip and run the exe.

Dark mode follows your Windows setting.

![Dark mode with four panes and the new tab start page](docs/screenshot-dark.png)

## Getting started

1. Grab `Sakadu-<version>-windows-x64.zip` from [Releases](https://github.com/noddish/sakadu/releases/latest) and unzip it anywhere.
2. Run `sakadu.exe`.

- If Windows shows "Windows protected your PC" on first launch, click **More info → Run anyway**. The exe isn't code-signed yet.
- Whole-disk search needs a small background service. Press **검색 서비스 설치** (install search service) in the search panel (Ctrl+F) once and approve the admin prompt.
- To open folders in Sakadu by default, open Settings (gear) → **일반** (General) and turn on **기본 파일 관리자로 사용** (use as default file manager).
- To keep the search hotkey working after you close the window, turn on **트레이에서 대기** (stay in tray) in the same place. Sakadu then also starts at sign-in, without a window.
- Theme, accent color, font and size live in Settings → **화면** (Appearance); shortcuts in Settings → **단축키** (Shortcuts). Press F1 for usage tips.
- Built and tested on Windows 11. It's meant to run on Windows 10 (64-bit) too, but that hasn't been tested yet.

## Keys you'll use most

| Key | What it does |
|---|---|
| Ctrl+F | Search the whole disk |
| Ctrl+K | Command menu: go to an open tab, favorite or recent location, or run a command |
| Just start typing | Filter the current folder (Esc clears) |
| Ctrl+1 – Ctrl+5 | Change the pane layout |
| Tab / Shift+Tab | Next / previous pane |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Alt+P | Preview pane |
| Ctrl+B / Ctrl+D | Open the favorites panel and go to it / add to favorites |
| Ctrl+Z | Undo the last file operation |

<details>
<summary>All shortcuts</summary>

| Key | What it does |
|---|---|
| Typing | Filter the current pane (Esc clears) |
| Enter / Ctrl+Enter | Open / open folder in a new tab |
| Backspace, Alt+↑ | Parent folder |
| Alt+← / Alt+→, mouse back/forward buttons | Back / forward |
| Tab / Shift+Tab | Switch pane |
| Ctrl+T / Ctrl+W (Ctrl+F4) / Ctrl+Tab | New tab / close tab / next tab |
| Ctrl+Shift+T | Reopen a closed tab (last 20) |
| Drag a tab | Reorder, or move to another pane |
| Drag from empty space or the date column | Select with a rectangle (hold Ctrl to add to the selection) |
| Ctrl+1 – Ctrl+5 | Pane layout |
| Ctrl+Shift+1 ~ Ctrl+Shift+6 | Extra large, large, medium, small icons, list, details (same as Explorer) |
| Alt+P | Preview pane |
| F2 / F5 | Rename / refresh |
| F4, Ctrl+L, Alt+D | Edit the path |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste (works on files even while the filter has text) |
| Ctrl+Shift+C | Copy the paths of the selected items as text (the current folder if nothing is selected) |
| Delete / Shift+Delete | Recycle Bin / delete permanently (asks first) |
| Ctrl+Shift+N | New folder |
| Ctrl+H | Show hidden items |
| Ctrl+F | Whole-disk search |
| Ctrl+K | Command menu (filter by name or Korean initial consonants; Enter current pane · Alt+Enter other pane · Ctrl+Enter new tab) |
| Ctrl+Z | Undo the last file operation (same as the **되돌리기** (undo) button on the notice after a move, copy or delete) |
| Ctrl+B | Open the favorites panel (pinned) and move the keyboard to it (↑↓, Enter, F2, Del, Ctrl+↑↓, ←→ collapse/expand groups, type to jump, Esc back to the pane) |
| Ctrl+D | Add to favorites (the selected folder or file, or the current folder) |
| Alt+Enter | Properties |
| Menu key, Shift+F10 | Context menu |

</details>

## Good to know

<details>
<summary>What the search service reads</summary>

- It reads **only file and folder names** from the NTFS file table (MFT) and keeps them in memory. File contents are never read. Changes come in live through the USN journal.
- Nothing is written to disk, and neither the app nor the service has any network code, so nothing leaves your PC.
- Reading the MFT needs admin rights, so it runs as a Windows service (SakaduIndex). The app talks to it over a local named pipe that refuses remote clients.
- The index takes about 120 MB of memory for 1.8 million entries.
- Results only include what the searching Windows account can see: every folder from the drive root down to the file's folder must be listable by that account (what you could browse in Explorer). Hidden files are left out of both the results and the total count. See [docs/search-permissions-check.md](docs/search-permissions-check.md) for a two-account check (Korean).
- Query syntax: space-separated words must all match. A word ending in `\`, like `work\`, matches the path. `|` means "or" (`quote 2026 | contract`). Pasting several lines joins them with `|`.
- Search opens as a **panel** on the left, so the results stay while you work in the panes. Esc only moves the keyboard back to the pane; close the panel with the search icon on the bar or its ×. Reopen it after closing and your last query is still there, selected: type to replace it, or press End to keep going.
- Results: a single click picks a result (and shows it in the preview pane if it is open), a double click or Enter opens it. Each result shows its modified date and size, so the newest of several same-named files is easy to spot. Ctrl+Enter opens the containing folder in the current pane with the file selected, Shift+Enter opens in a new tab. Pick several with Ctrl+click, Shift+click or Shift+↑↓, then Enter opens them all and Ctrl+C copies the files. The right-click menu also has copy path and pick all.
- The **지금 폴더** (current folder) chip limits the search to the folder shown in the active pane. When that folder changes, or files are moved or renamed, the same query runs again so the results stay current.
- The global hotkey is the first free one of Ctrl+Alt+Space, Win+Shift+F and Ctrl+Alt+F. Hover over the search icon on the left bar to see which one you got.
- The service exe is copied to `C:\Program Files\Sakadu`. After updating Sakadu, reinstall the service to update it too.

</details>

<details>
<summary>Searching shared folders and USB drives</summary>

The search service only reads this PC's own (NTFS) disks. Shared folders (`\\server\share`), NAS and USB drives are searched once you add them.

- Open the folder in any pane and search with Ctrl+F. If nothing turns up, click **이 폴더도 검색하기** (search this folder too) in the search panel.
- The **검색에 넣은 폴더** (added folders) section of the 바로가기 (places) panel on the left lists them. When the current pane shows a folder outside the search, the **+** in that section adds it; right-click a folder to remove it.
- Settings → **검색** (Search) → **검색에 넣은 폴더** also lists them, lets you remove one, and adds folders you have open.
- The app walks the folder once with your own account and keeps the names in memory. Nothing is written to disk, so it walks the folder again each time the app starts.
- Changes come in through folder change notifications. In case a server doesn't send them, the folder is walked again every few hours, and opening it in Sakadu also brings it up to date.
- If the connection drops, the last results stay searchable and Sakadu tries to reconnect every 5 minutes.
- The app has to be running to keep up, so this pairs well with **트레이에서 대기** (stay in tray).
- When a share needs a sign-in, the list shows a **로그인** (sign in) button. It opens the Windows network sign-in dialog (Windows takes the account and password) and reads the folder again once connected. A disconnected mapped drive (Z: and so on) gets a **다시 연결** (reconnect) button.
- If an open shared folder loses its connection, the last list seen in this session stays visible, dimmed, and refreshes by itself once the connection is back.

</details>

<details>
<summary>Using it as the default file manager</summary>

When turned on in Settings → General, opening folders and drives, and Win+E, go to Sakadu.

- Only your user settings (HKCU) change. The original values are backed up and restored when you turn it off.
- Explorer caches the old Win+E command, so Win+E may only switch after you sign out and back in or restart. Opening folders switches right away.
- The desktop, taskbar and special folders like Control Panel and Recycle Bin still open in Explorer.
- If Sakadu is already running, the folder opens in a new tab. If that window isn't responding, the folder opens in Explorer instead so it never fails to open.
- The exe is copied to `%LOCALAPPDATA%\Programs\Sakadu`. Running a newly downloaded `sakadu.exe` once updates that copy too.
- Command line: `sakadu.exe --register-default` / `--unregister-default`

</details>

<details>
<summary>Staying in the tray</summary>

When turned on in Settings → General, closing the window doesn't quit Sakadu. It stays as an icon in the notification area (tray).

- The global hotkey brings back the window with search open. Clicking the tray icon brings the window back too.
- It's added to your user's startup programs (HKCU Run) so it starts at sign-in without a window. Turning it off removes that entry.
- To quit completely, right-click the tray icon and choose **끝내기** (quit).
- While hidden it uses about 30 MB of memory. If an open folder changes, Sakadu doesn't reread it until the window comes back.
- The exe is copied to `%LOCALAPPDATA%\Programs\Sakadu`, the same place the default file manager option uses. Running a newly downloaded `sakadu.exe` quits the older one waiting in the tray, takes its place, and updates that copy. If version 0.5.1 or older is running, Sakadu asks you to quit it from the tray first.

</details>

<details>
<summary>Thumbnails, preview, favorites, undo</summary>

- The **view button** at the right end of the path row picks extra large, large, medium or small icons, list, or details. Shortcuts match Explorer (Ctrl+Shift+1–6), and each pane remembers its own. Icon views show thumbnails for photos and PDFs.
- **New tabs** (Ctrl+T, +) open on a **start page** ("어디로 이동할까요?", where to?): favorite folders as cards, recently visited folders as rows, then This PC and the drives. A click opens it in that tab (Ctrl+click or middle click opens a new tab); type to filter by name and press Enter to go. Arrow keys move between them, and right-click or Delete removes a recent folder from the list. Change it in Settings → General → **새 탭을 열 위치** (new tab location): This PC, the same folder as the current tab, or a folder of your choice.
- The **star** button right of the path adds the current folder to favorites in one click (the star fills in). Click again to remove it. The button next to it makes a **new folder**. In a narrow pane the back/forward buttons and then the star make room so the current folder name stays visible (Alt+←→ and Ctrl+D still work). Drop files on a path segment to put them in that folder.
- Hover over a button on the work bar (split, `…`, hidden items, preview) or on the icon bar at the left for a moment to see its name and shortcut. Help, settings and about are under `…` at the right of the work bar.
- Each pane shows its tabs as chips along the top, with the item count and the selection's count and size on the right. Typing turns that spot into a **filter chip**; × or Esc clears it. With several panes, a thin accent line marks the top of the current one.
- Modified dates read short: `방금` (just now), `오늘 10:24` (today), `어제 18:40` (yesterday), `11월 3일` this year, and `2025-03-04` before that. Settings → 화면 → **날짜** switches back to the full date and time.
- The **icon bar** on the left holds places, favorites, search and settings. When no panel is pinned, resting the mouse on an icon opens its panel over the panes; it closes when the mouse leaves or you pick a location. While a panel is pinned, the other icons switch only when clicked. Click the icon, or the pin in the panel, to keep it open: it then pushes the panes aside and its right edge can be dragged to change the width. Click the icon again to close it and give the space back to the files.
- **Workspaces**: the button at the left of the work bar (`작업 공간 ▾` at first, then the current workspace's name) starts a new workspace from the current screen and switches between workspaces. A workspace keeps the split, panes and tabs, the pinned panel, the preview and its pinned file, and the search query and scope. The current workspace follows the screen without a separate save (it is saved when you switch, close the window or shut down Windows), and Sakadu reopens on the last screen. Rename and delete are in the same menu.
- The **바로가기** (places) panel shows This PC, drives (with a free-space bar) and added search folders. Drop files on a drive or folder row to put them there (same rule as drag and drop below). Click a section header (drives, search folders) to fold or unfold it; folded sections stay folded.
- **Preview pane** (Alt+P) shows photos large and the start of text and code files (UTF-8, UTF-16 and CP949 detected). PDF and Office files use the preview handlers registered in Windows, run out of process so a crashing handler can't take the app down. Photos also show their width × height.
- The **pin** in the preview header (or the `선택 따라가기` chip) **pins** the file (`파일 고정`): it stays in the preview while you select other files, and follows the file when it is renamed or moved to another folder on the same NTFS drive, even from Explorer. If it is deleted or moved where it can't be found (another drive, a network share), the pin is released and the preview follows the selection again. The button next to the pin opens the file in its default app.
- **Favorites** can be folders or files (add with the star, Ctrl+D, or by dragging from a file list onto the panel or the star icon in the bar), with their own display names like "01. Daily work", independent of the real name (F2). Drag to reorder. Click a **group** to fold or unfold it in place; drag an item onto a group, or right-click → **그룹으로 이동** (move to group). Clicking a file opens it, and Ctrl+click (Ctrl+Enter from the keyboard) opens its folder in a new tab. Ctrl+B moves the keyboard to the favorites. Rest the mouse on an item to see a card with its name, type and path (and a small picture for photos). The add, sort and new-group buttons in the panel header appear while the mouse is over the panel or the keyboard is in the favorites.
- **Rename** (F2) selects the name without the extension. Characters Windows doesn't allow (\ / : * ? " < > |) can't be typed, and changing the extension asks first. Clicking another pane or the path also confirms the name. Making a new folder while the filter has text clears the filter first.
- **Undo** (Ctrl+Z) reverses renames, moves, copies, Recycle Bin deletes, new folders, pastes and drag and drop, newest first. Permanent deletes and actions from the context menu can't be undone, and undo waits until a running copy or move finishes.
- **Drag and drop** works like File Explorer: within the same drive (or the same share) files move, and to another drive, share or USB stick they are copied. Hold Shift to move, Ctrl to copy, and Ctrl+Shift or Alt to create a shortcut; the label next to the cursor shows which one will happen. Dropping files back into the folder they came from does nothing (so a small accidental drag never makes a copy); hold Ctrl to make a copy there.
- Opening more than 15 files at once asks first. Opening an offline NAS path or favorite never freezes the window; `여는 중…` (opening…) shows while it is checked. When a folder takes longer than 0.3 s to read, `불러오는 중…` (loading…) appears in the list, and F5 retries a folder that couldn't be opened.

</details>

<details>
<summary>Settings and shortcuts</summary>

The gear button (Ctrl+,) opens Settings over the whole window. Pick a category on the left; Esc or ← closes it.

- **일반** (General): new tab location, hidden items, preview pane, default file manager, stay in tray
- **화면** (Appearance): theme (system, light, dark), accent color (or follow the Windows accent), font, size (90–150%)
- **검색** (Search): install or remove the search service, added folders
- **단축키** (Shortcuts): click a row and press the new keys. If another action used them, that one is cleared. Esc cancels, Backspace clears. The anywhere-search hotkey is set here too.
- **도움말** (Help, F1): search syntax and everyday tips
- **정보** (About): version, GitHub, settings folder

Only changed shortcuts are saved, in `%APPDATA%\Sakadu\keys.txt`. Basic keys such as arrows, Enter, Esc, Tab and Ctrl+C/X/V/A can't be remapped.

</details>

<details>
<summary>Uninstalling</summary>

1. In Settings → General, turn off **기본 파일 관리자로 사용** (default file manager) and **트레이에서 대기** (stay in tray). Then in Settings → Search, click **제거…** (remove) next to the search service.
2. Delete the unzipped folder, `%APPDATA%\Sakadu`, and `%LOCALAPPDATA%\Programs\Sakadu` if it exists.

Settings, window state and favorites all live in `%APPDATA%\Sakadu`.

</details>

## Found a problem?

Please open an [issue](https://github.com/noddish/sakadu/issues). For something hard to reproduce, run `sakadu.exe --log`
and attach `%TEMP%\sakadu.log`. The log contains folder paths, so give it a look before posting.

When Sakadu runs as administrator (the window title on the taskbar and in Alt+Tab says "(관리자)"), Windows (UIPI) blocks drag and drop with a normal Explorer window.

## Building

You need stable Rust (MSVC toolchain, edition 2024) and the Windows SDK.

```
cargo build --release
cargo test
```

<details>
<summary>Dev notes and code layout</summary>

- If the project path is very long, the linker can hit the 260-character limit (LNK1104). Point only the build output somewhere short:
  `$env:CARGO_TARGET_DIR = "$env:TEMP\sakadu-target"; cargo build --release`
- Tests that touch the real shell or service, and benchmarks, are `#[ignore]`d: `cargo test --release -- --ignored --nocapture`
- Force a theme: `SAKADU_THEME=dark` or `light`
- Running a release build by hand, with "stay in tray" or "default file manager" on, replaces the copy in `%LOCALAPPDATA%\Programs\Sakadu` with that build and takes over from a different build that's running (debug builds don't). When launching test windows, point `LOCALAPPDATA` to another folder along with `APPDATA`.
- With `SAKADU_ISOLATED=1`, a test window runs on its own next to the Sakadu you are using: it uses its own window class and skips the single-instance hand-off, take-over, install-copy update, stay-in-tray and the global hotkey.
- Icons live in `assets/*.ico`. `build.rs` embeds them in the exe without a resource compiler.
- Releases: bump `version` in `Cargo.toml` and push a tag like `v0.2.0`. GitHub Actions builds it and creates a **draft** release to check and publish.

| File | Role |
|---|---|
| `main.rs` | Window creation, message loop, re-entrancy, command line, single instance (takes over from a different running version) |
| `app.rs` | Layout, input, drawing, pane and tab behavior |
| `app/settings_ui.rs` | Settings window (general, appearance, search, shortcuts, help, about) |
| `keys.rs` | Remappable shortcuts (key map, saving) |
| `app/sidebar.rs` · `app/fav_ui.rs` | Icon bar and panels on the left (places, favorites; peek, pin, resize) · favorites actions |
| `app/palette.rs` · `app/preview.rs` | Ctrl+F search panel · preview pane |
| `app/commands.rs` | Ctrl+K command menu (open tabs, favorites, recent locations, workspaces, commands) |
| `app/start.rs` | New tab start page (favorite folder cards, recent locations, drives) |
| `app/workspace_ui.rs` · `workspaces.rs` | Workspace button, menu and switching · file format |
| `pane.rs` | Tab state (history, sort, filter, selection) |
| `listing.rs` | Folder listing, This PC, display formatting |
| `shell.rs` | Shell menus, file operations, clipboard, drag, change notifications, icons |
| `droptarget.rs` | Drop target (delegates to the shell's drop target) |
| `thumbs.rs` | Background thumbnail generation |
| `index.rs` · `service.rs` | NTFS name index (MFT scan, USN updates, ranking) · search service, pipe, install |
| `folders.rs` | Added folders (shares, USB): walking, change tracking, search |
| `defaultfm.rs` | Default file manager register/unregister (HKCU, backup and restore), copying and updating the installed copy |
| `tray.rs` | Staying in the tray (notification icon and menu, start at sign-in) |
| `undo.rs` | Undo history (shell progress sink) |
| `favorites.rs`, `session.rs` | Saving favorites and sessions |
| `gfx.rs`, `theme.rs`, `util.rs`, `diag.rs` | Rendering, colors, paths, diagnostic log |

Contributions are welcome. For bigger changes, please open an issue first. PRs need `cargo test` to pass.

</details>

## The name

Sakadu comes from *saccade*, the quick jump your eyes make from one point to another.
Getting to the file you want should feel that quick.

## License

[GPL-3.0-or-later](LICENSE). If you distribute a modified version, you need to share its source under the same license.
