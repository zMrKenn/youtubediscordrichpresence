# youtube-rpc

A small native Windows app that shows what you're playing on YouTube as your Discord
status — title, channel, a live progress bar, the thumbnail, and two buttons. It lives in
the tray with a little control window drawn in Rust (egui, no webview). Pairs with the
browser extension in `../extension`.

## How it works

```
Browser extension  ──HTTP──▶  youtube-rpc.exe  ──named pipe──▶  Discord desktop
(reads the player)  :41414     (tray + window)     IPC             (your profile)
```

The extension reads the current video straight from YouTube's player and POSTs it to
`127.0.0.1:41414`. This app picks it up and talks to the Discord desktop client over its
local IPC pipe (`\\.\pipe\discord-ipc-N`). No bot, no login, no tokens.

## The window

Open it from the tray icon (right-click → **Open**, or double-click it). Launching the exe
a second time also brings the existing window forward instead of starting a duplicate. It
shows the song currently on your profile, a **Show as** switch (Listening / Watching /
Playing), a **Start with Windows** toggle, and a connection dot.

- **Double-clicking the exe** opens the window.
- **Started at login** (autostart adds a `--startup` flag) it stays hidden in the tray.
- **Closing with X** drops it to the tray and keeps running — the first time, a one-off
  notice says so.
- **Quit** from the tray menu stops it for good.

## String obfuscation

Every meaningful string literal — URLs, button labels, Discord IPC commands, the Discord
app id, config keys, window text — is XOR-obfuscated with `obfstr` and only decrypted on
the stack at runtime, so a `strings youtube-rpc.exe` dump turns up nothing useful.

That's it though: it stops casual snooping (strings dumps, hex editors), not a determined
reverse engineer with a debugger — the program has to decrypt the strings to use them.
Nothing that actually runs can be fully encrypted. The `.exe` is already closed-source
anyway (compiled, symbols stripped).

## Build and run

```bash
cargo build --release
```

You get a single self-contained `target/release/youtube-rpc.exe` (~4 MB, no runtime
dependencies, no console window). Double-click it — a red play icon shows up in the tray.
Right-click → **Start with Windows** to launch it at login.

The Discord app id is baked into the binary, so the exe works on its own with no config
file. Drop a `.env` next to it only if you want to override something:

```
DISCORD_APP_ID=your_own_app_id   # use your own Discord application instead
BRIDGE_PORT=41414                # must match the extension
ACTIVITY_TYPE=2                  # 2 = Listening, 3 = Watching, 0 = Playing
SMALL_ICON_URL=                  # optional small corner badge (public image URL)
```

If you use your own app id, name that application **YouTube** in the Discord Developer
Portal — the name is what shows after "Listening to …".

## Notes

- **The buttons don't show on your own profile** — Discord only shows them to other people.
  Check from a friend's account or Discord mobile.
- The debug build (`cargo build`) keeps a console window, handy if something misbehaves.
- Presence clears on its own about 30 seconds after you close the YouTube tab, and the app
  reconnects by itself if you restart Discord.
