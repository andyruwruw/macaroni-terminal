# Browser tabs (system web view or Chromium)

## Mental model

A browser tab is `TabKind::Browser(BrowserId)` with its address in `Tab.url`. The page is a **native child view** of macaroni's window. iced never draws it; iced lays out an empty `BrowserHost` placeholder and the app moves the page over it.

Two engines sit behind `macaroni_browser::Browser` (an enum over both), picked per page by `[browser] engine` when the page starts:

- **System** (default; `crates/browser/src/system.rs`): the OS web view through `wry`: WebView2 (Edge) on Windows, WKWebView (Safari) on macOS. Nothing ships with the app. Runs entirely on the main thread.
- **Chromium** (Windows only; `chromium.rs`): CEF with its own UI thread, not shipped with the app but downloaded on request (see below). On macOS `Engine::Chromium` falls back to the system engine.

```
Effect::OpenBrowser(id) ──► App.browsers[id] = Panel (no browser yet)
BrowserHost(id) ──► Message::BrowserBounds(id, rect) on change
every update ──► sync_browsers():
    on screen + bounds known + not started ──► Browser::open(engine, ...) (starts CEF once for Chromium)
    otherwise ──► Browser::place(Some(rect)) or place(None) to hide
engine callbacks ──► handler/shortcut closures ──► (BrowserId, BrowserMessage) channel ──► Message::Browser
Effect::CloseBrowser(id) ──► drop Panel ──► Browser::drop closes it
```

- **Lazy.** A browser starts the first time its tab is shown, at its real size. `Browser::open` returns an error (shown in the tab) if the engine can't start.
- **Threads.** System pages are created, moved and called on the main thread, and their handlers run there (`Rc`, no locks). CEF runs its own UI thread (`multi_threaded_message_loop`); its placement uses `SetWindowPos` with `SWP_ASYNCWINDOWPOS` so the iced thread never waits on it, and state is shared through `Shared` behind a mutex. Both skip redundant placements and hide in place (keeping the size).
- **One executable.** Chromium subprocesses are `macaroni.exe --type=...`; `macaroni_browser::helper_process(&chromium::dir())` must run first in `main` and exits for them (it loads the runtime first; without `--type=` it returns at once, loading nothing). Scripts looking for "the macaroni window" must pick the titled process. System pages run in the OS's own processes (`msedgewebview2.exe`).
- **What `wry` doesn't report** comes from `system::SCRIPT`, injected into each page's main frame: in-page navigation (`pushState`, `popstate`, `hashchange`) and `window.close()` as IPC messages, plus focus and modified keys on macOS. On Windows focus and keys come from WebView2's controller events (`system::native`): `GotFocus` and `AcceleratorKeyPressed`.
- **Keyboard.** While a page has focus macaroni gets no key events. Each engine offers keys to `browser::shortcuts`, which checks `App.shared_keymap` (an `Arc<Mutex<Keymap>>` refreshed on config reload); a match becomes `BrowserMessage::Action`. CEF (`on_pre_key_event`) and WebView2 (`AcceleratorKeyPressed`, which covers every key with Ctrl or Alt and every non-character key) swallow a matched key. On macOS the script reports keys after the page saw them, so a shortcut there isn't kept from the page. When a browser tab is focused but macaroni's window has the keyboard, the `unhandled_keys` subscription handles shortcuts.
- **Focus.** `App.page_focus` is the browser holding the keyboard. A page click (`Event::Focused`) selects its tab; a click in macaroni's UI (`clicks` subscription) and switching to a terminal tab call `App::blur_page`, which calls `Browser::blur` (focus back to the parent window); switching to a browser tab calls `Browser::focus`, as does a page started by switching to its tab. `window_focused` stays true while a page has focus, because Windows reports focus moving into the child as the app losing it.
- **Links and popups.** `open_link` sends terminal links to the workspace's browser pane (`open_in_browser_pane`) or a new split, per `[browser] open_links`. `target=_blank` and `window.open` become `OpenBrowserBeside` tabs. With Chromium, `window.open` with features (`NEW_POPUP`) opens a real top-level window with its own `PopupClient` (no app shortcuts), counted in `LIVE` and tracked in `Shared.popups` so closing the tab closes them; the system engine opens those as tabs too, so they lose `window.opener`.

## The Chromium download (`crates/app/src/chromium.rs`)

- The app is linked with `/DELAYLOAD:libcef.dll` (`crates/app/build.rs`), so it starts without Chromium. `chromium::load` (in the browser crate) loads `libcef.dll` with `LoadLibraryExW` + `SetDllDirectoryW` from `runtime_dir` before any CEF call, then calls `api_hash`. Order matters: any CEF call before the library is loaded makes the delay-load look next to the exe; any before `api_hash` stops the process at a check (exit `0x80000003`).
- `runtime_dir(download)`: the download folder (`%LOCALAPPDATA%\macaroni\chromium`, or `<MACARONI_CONFIG_DIR>/chromium`) when its `archive.txt` names this build's archive, else the exe's folder when `libcef.dll` is there (dev builds, where `crates/browser/build.rs` copies the runtime). `cargo xtask bundle` doesn't ship the runtime, only `Chromium-CREDITS.html` (CEF's wrapper is linked in).
- The build pins the archive: `crates/browser/build.rs` reads `archive.json` from the CEF distribution and sets `MACARONI_CEF_ARCHIVE` / `MACARONI_CEF_SHA1`, exported as `macaroni_browser::CHROMIUM_ARCHIVE` / `CHROMIUM_SHA1`.
- Settings > Integrations > Browser > Download runs `chromium::install` off the UI thread: System32 `curl` fetches `https://cef-builds.spotifycdn.com/<archive>` (`+` as `%2B`; about 175 MB), `certutil -hashfile` checks the SHA-1, and System32 `tar` (bsdtar, which reads bz2) extracts `<root>/Release` and `<root>/Resources` with `--strip-components 2` into the folder, skipping `*.lib` and the sandbox bootstrap. `archive.txt` is written last. Remove deletes the folder, refused while this process has Chromium loaded (its files are locked).
- A macaroni built against a newer CEF ignores an old download (its `archive.txt` doesn't match), so the row offers Download again.

## VS Code tabs (`crates/app/src/vscode.rs`)

- Opt-in: nothing ships with the app. Settings > Integrations > VS Code > Download runs `vscode::install` on a thread (`off_thread`): Windows' own `System32\curl.exe` fetches Microsoft's standalone VS Code CLI zip and `System32\tar.exe` (bsdtar, which reads zips; Git's GNU tar doesn't) unpacks it into `vscode::dir()` (`%LOCALAPPDATA%\macaroni\vscode`, or `<MACARONI_CONFIG_DIR>/vscode`). Remove stops the server and deletes the folder.
- A VS Code tab is an ordinary browser tab (either engine) at `Server::url(folder)`: `code serve-web` on 127.0.0.1 with a connection token (`?tkn=`) and `folder=` as a URI path (`/C:/...`). The CLI downloads the server itself on the first load. `App.vscode_web` (`WebVsCode`) holds the server; it starts on the first tab, and tabs asked for before it listens wait in `waiting` until `Message::VsCodeReady` (the `vscode_listening` subscription polls the port).
- Port and token are saved in `vscode/server`, so restored tabs' addresses still work. `start_browser` holds back a tab whose address is the server's (`WebVsCode::owns`) until it listens, starting it if needed.
- Dropping `Server` runs `taskkill /T /F` on the CLI: killing the CLI alone leaves the server running on the port. `App::exit` stops it. Logs go to `vscode/server.log`.

## Files

- `crates/browser/` (`macaroni-browser`): the only crate that touches a web engine. `src/lib.rs` (`Engine`, the `Browser` enum dispatching with `each!`, `helper_process`, `wait_closed`, `shutdown`); `src/system.rs` (`wry`, `SCRIPT`, WebView2's `native` events); `src/chromium.rs` (`runtime_dir`, `load`, `RUNTIME`, `start`, `shutdown`, `Browser`, the `wrap_*!` handlers, Win32 placement and library loading). CEF is only a dependency on Windows; `wry` on Windows and macOS. `build.rs` copies the CEF runtime next to the executables for dev builds and records the archive name and SHA-1.
- `crates/app/src/chromium.rs`: `install`, `remove`, `dir`, `installed`, the `Chromium` state shown in the settings row (`chromium_setting` in `view/settings/list.rs`; `chromium_install` / `chromium_installed` in `update/menus.rs`).
- `crates/app/src/browser.rs`: `Panel`, `BrowserHost`, `engine` (config to `Engine`), `resolve` (address bar input to URL or search), `shortcuts`, `prefer_gl`, the profile directory (`data_dir`).
- `crates/app/src/update/browser.rs`: `sync_browsers` (`place_browsers`, `follow_focused_tab`), `blur_page`, `start_browser`, `browser_message`, `open_link`, `open_in_browser_pane`. `exit` is in `update/effects.rs`.
- `crates/app/src/view/panes.rs` (`browser_body`, the address bar), `view/tabs.rs` (`tab_title`).
- `crates/app/macaroni.exe.manifest`, embedded by `crates/app/build.rs`.

## Building

- `cef-dll-sys` builds CEF's C++ wrapper with CMake and Ninja. The cmake crate finds Visual Studio's CMake, but Ninja must be on `PATH`. `cargo xtask dev` / `bundle` add VS's Ninja (via `vswhere`); plain `cargo build` doesn't.
- `.cargo/config.toml` sets `CEF_PATH = target/cef` so the CEF download is reused across clean builds.
- Keep the repo at a short path: CMake/Ninja fail past MAX_PATH ("cannot find CMakeFiles\rules.ninja").

## Traps

- **iced must render with GL on Windows.** DWM composites flip-model swap chains (wgpu DX12/Vulkan) above child windows, so the page is present but invisible. `browser::prefer_gl()` sets `WGPU_BACKEND=gl` before anything else unless the user set it. This applies to WebView2's child window as much as CEF's.
- **WebView2 takes focus back into a page hidden while it has it** (`GotFocus` fires after the hide), which would select the page's tab again. `place_browsers` calls `blur_page` before hiding the focused page. The page's own `focus` event is worse (it fires after app shortcuts too), so Windows uses `GotFocus`, never the script's.
- **`chromium.rs` glob-imports `cef::*`.** Its own `start` / `shutdown` / `wait_closed` shadow CEF's names: `Runtime::drop` must call `cef::shutdown()` spelled out, or it calls the module's `shutdown`, which locks `RUNTIME` while it's held and hangs on quit.
- **The manifest must be embedded.** Without a Windows 10 `supportedOS` entry Windows reports version 8 and Chromium's GPU process dies at start (pages fall back to software); a side-by-side `.manifest` isn't enough.
- **Testing a close with `taskkill`** (no `/F`) can leave a Chromium build running when its window was brought to the front first: the `WM_CLOSE` doesn't reach the main window, which stays responsive. Alt+F4 and the title bar close it normally; close test instances some other way, or check `Responding` before calling it a hang.
- **One Chromium per profile.** The profile is `%LOCALAPPDATA%\macaroni\browser`, or `<MACARONI_CONFIG_DIR>/browser`. A second instance on the same profile fails `Runtime::start` and the tab shows the error; use `MACARONI_CONFIG_DIR` for test instances. WebView2 keeps its data in `<profile>/system` and can share it between processes; WKWebView uses the app's default data store.
- **Closing a tab's page.** A panel's page is a child of the app window. CEF's default `DoClose` (return 0) sends WM_CLOSE to the page's top-level window, which is the app's, so closing one browser tab quit the app. The panel `LifeSpan::do_close` destroys the page's own window and returns 1; `on_before_close` follows. Popups keep the default (they're top-level).
- **Shutdown order.** `App::exit` drops every browser and waits off the UI thread for CEF to close them (`LIVE`) before `iced::exit()`; `browser::shutdown()` runs `cef::shutdown` on the main thread after iced returns.
- **Nothing iced draws can overlap a page.** It's a native window above iced's surface, so `place_browsers` hides a page while an iced menu covers it or the settings dialog is open.
- `read_page` is async (CEF's `Frame::text` visitor, or `document.body.innerText` evaluated in the system view): the IPC handler stores the request in `App.text_requests` and answers on `BrowserMessage::Text`.
- Screenshots: `PrintWindow` misses the page; capture the screen area.
- Windows and macOS. Elsewhere `Browser::open` fails and `open_link` sends links to the system browser. The macOS side (WKWebView placement, the script's focus and keys) type-checks but has only been run on Windows.
