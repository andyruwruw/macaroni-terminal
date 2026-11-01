# Browser tabs (Chromium via CEF)

## Mental model

A browser tab is `TabKind::Browser(BrowserId)` with its address in `Tab.url`. The page is Chromium via CEF (the `cef` crate) as a **native child window** of macaroni's window. iced never draws it; iced lays out an empty `BrowserHost` placeholder and the app moves the Chromium window over it.

```
Effect::OpenBrowser(id) ──► App.browsers[id] = Panel (no browser yet)
BrowserHost(id) ──► Message::BrowserBounds(id, rect) on change
every update ──► sync_browsers():
    on screen + bounds known + not started ──► browser::start() (CEF, once) + Browser::open
    otherwise ──► Browser::place(Some(rect)) or place(None) to hide
CEF UI thread ──► handler/shortcut closures ──► (BrowserId, BrowserMessage) channel ──► Message::Browser
Effect::CloseBrowser(id) ──► drop Panel ──► Browser::drop closes it
```

- **Lazy.** A browser starts the first time its tab is shown, at its real size.
- **Threads.** CEF runs its own UI thread (`multi_threaded_message_loop`). Placement uses `SetWindowPos` with `SWP_ASYNCWINDOWPOS` so the iced thread never waits on CEF. Hiding keeps the size so hidden pages don't relayout.
- **One executable.** Chromium subprocesses are `macaroni.exe --type=...`; `macaroni_browser::helper_process()` must run first in `main` and exits for them. Scripts looking for "the macaroni window" must pick the titled process.
- **Keyboard.** While a page has focus macaroni gets no key events. `on_pre_key_event` offers each keydown to `browser::shortcuts`, which checks `App.shared_keymap` (an `Arc<Mutex<Keymap>>` refreshed on config reload); a match becomes `BrowserMessage::Action` and is swallowed. When a browser tab is focused but macaroni's window has the keyboard, the `unhandled_keys` subscription handles shortcuts.
- **Focus.** `App.page_focus` is the browser holding the keyboard. A page click (`Event::Focused`) selects its tab; a click in macaroni's UI (`clicks` subscription) calls `focus_window(parent)` to take it back; switching to a browser tab programmatically calls `Browser::focus`. `window_focused` stays true while a page has focus, because Windows reports focus moving into the child as the app losing it.
- **Links and popups.** `open_link` sends terminal links to the workspace's browser pane (`open_in_browser_pane`) or a new split, per `[browser] open_links`. `target=_blank` and plain `window.open` become `OpenBrowserBeside` tabs. `window.open` with features (`NEW_POPUP`) opens a real top-level window with its own `PopupClient` (no app shortcuts), counted in `LIVE` and tracked in `Shared.popups` so closing the tab closes them.

## VS Code tabs (`crates/app/src/vscode.rs`)

- Opt-in: nothing ships with the app. Settings > Integrations > VS Code > Download runs `vscode::install` on a thread (`off_thread`): Windows' own `System32\curl.exe` fetches Microsoft's standalone VS Code CLI zip and `System32\tar.exe` (bsdtar, which reads zips; Git's GNU tar doesn't) unpacks it into `vscode::dir()` (`%LOCALAPPDATA%\macaroni\vscode`, or `<MACARONI_CONFIG_DIR>/vscode`). Remove stops the server and deletes the folder.
- A VS Code tab is an ordinary browser tab at `Server::url(folder)`: `code serve-web` on 127.0.0.1 with a connection token (`?tkn=`) and `folder=` as a URI path (`/C:/...`). The CLI downloads the server itself on the first load. `App.vscode_web` (`WebVsCode`) holds the server; it starts on the first tab, and tabs asked for before it listens wait in `waiting` until `Message::VsCodeReady` (the `vscode_listening` subscription polls the port).
- Port and token are saved in `vscode/server`, so restored tabs' addresses still work. `start_browser` holds back a tab whose address is the server's (`WebVsCode::owns`) until it listens, starting it if needed.
- Dropping `Server` runs `taskkill /T /F` on the CLI: killing the CLI alone leaves the server running on the port. `App::exit` stops it. Logs go to `vscode/server.log`.

## Files

- `crates/browser/` (`macaroni-browser`): the only crate that touches CEF. `src/windows.rs` (`Runtime`, `Browser`, the `wrap_*!` handlers, Win32 placement); `src/unsupported.rs` stub elsewhere (CEF isn't a dependency off Windows). `build.rs` copies the CEF runtime next to the executables.
- `crates/app/src/browser.rs`: `Panel`, `BrowserHost`, `resolve` (address bar input to URL or search), `shortcuts`, the `RUNTIME` static, `prefer_gl`, `shutdown`, the profile directory.
- `crates/app/src/update/browser.rs`: `sync_browsers` (`place_browsers`, `follow_focused_tab`), `start_browser`, `browser_message`, `open_link`, `open_in_browser_pane`. `exit` is in `update/effects.rs`.
- `crates/app/src/view/panes.rs` (`browser_body`, the address bar), `view/tabs.rs` (`tab_title`).
- `crates/app/macaroni.exe.manifest`, embedded by `crates/app/build.rs`.

## Building

- `cef-dll-sys` builds CEF's C++ wrapper with CMake and Ninja. The cmake crate finds Visual Studio's CMake, but Ninja must be on `PATH`. `cargo xtask dev` / `bundle` add VS's Ninja (via `vswhere`); plain `cargo build` doesn't.
- `.cargo/config.toml` sets `CEF_PATH = target/cef` so the CEF download is reused across clean builds.
- Keep the repo at a short path: CMake/Ninja fail past MAX_PATH ("cannot find CMakeFiles\rules.ninja").

## Traps

- **iced must render with GL on Windows.** DWM composites flip-model swap chains (wgpu DX12/Vulkan) above child windows, so the page is present but invisible. `browser::prefer_gl()` sets `WGPU_BACKEND=gl` before anything else unless the user set it.
- **The manifest must be embedded.** Without a Windows 10 `supportedOS` entry Windows reports version 8 and Chromium's GPU process dies at start (pages fall back to software); a side-by-side `.manifest` isn't enough.
- **One Chromium per profile.** The profile is `%LOCALAPPDATA%\macaroni\browser`, or `<MACARONI_CONFIG_DIR>/browser`. A second instance on the same profile fails `Runtime::start` and the tab shows the error; use `MACARONI_CONFIG_DIR` for test instances.
- **Closing a tab's page.** A panel's page is a child of the app window. CEF's default `DoClose` (return 0) sends WM_CLOSE to the page's top-level window, which is the app's, so closing one browser tab quit the app. The panel `LifeSpan::do_close` destroys the page's own window and returns 1; `on_before_close` follows. Popups keep the default (they're top-level).
- **Shutdown order.** `App::exit` drops every browser and waits off the UI thread for CEF to close them (`LIVE`) before `iced::exit()`; `browser::shutdown()` runs `cef::shutdown` on the main thread after iced returns.
- **Nothing iced draws can overlap a page.** It's a native window above iced's surface, so `place_browsers` hides a page while an iced menu covers it or the settings dialog is open.
- `read_page` is async (CEF's `Frame::text` visitor): the IPC handler stores the request in `App.text_requests` and answers on `BrowserMessage::Text`.
- Screenshots: `PrintWindow` misses the page; capture the screen area.
- Windows only. Elsewhere the `unsupported` stub reports that browser panels aren't supported.
