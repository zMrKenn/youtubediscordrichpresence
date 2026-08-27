# YouTube Presence for Discord

Shows what you're watching or listening to on YouTube as your Discord status —
song or video title, channel, a live progress bar, and the thumbnail. Works with
both youtube.com and music.youtube.com.

## How it works

Two pieces:

- **This browser extension** reads the current video straight from YouTube's own
  player, so it stays accurate even when songs auto-advance in a playlist or radio.
- **A small desktop app** receives that over `http://127.0.0.1` and shows it on your
  Discord profile through Discord's local connection. No bot, no login, no tokens.

```
extension  ──►  localhost:41414  ──►  desktop app  ──►  Discord
```

The extension does nothing on its own — you need the desktop app running for the
status to appear.

## Install

**From source (unpacked):**

1. Download or clone this folder.
2. Open `chrome://extensions` (or `brave://extensions`, `edge://extensions`).
3. Turn on **Developer mode**.
4. Click **Load unpacked** and pick this `extension` folder.
5. Install and run the companion desktop app, then play something on YouTube.

## Privacy

Everything stays on your machine. The extension only ever sends the current title,
channel, timestamps, and thumbnail URL to `127.0.0.1` (your own computer) — nothing
is sent anywhere else, and there are no accounts or trackers. It only runs on
YouTube pages.

## Not affiliated

This is an independent, open-source project. It is not affiliated with, endorsed by,
or sponsored by YouTube, Google, or Discord. "YouTube" and "Discord" are trademarks
of their respective owners.
