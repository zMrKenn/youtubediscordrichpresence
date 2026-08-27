<div align="center">

<img src="https://readme-typing-svg.demolab.com?font=JetBrains+Mono&weight=700&size=28&duration=2800&pause=1200&color=A78BFA&center=true&vCenter=true&width=760&lines=YouTube+Presence+for+Discord;What+you're+playing%2C+on+your+profile;Native+%C2%B7+no+bot+%C2%B7+no+login" alt="YouTube Presence for Discord" />

<sub><i>Like Spotify's status, but for YouTube.</i></sub>

<br/><br/>

<img src="https://img.shields.io/badge/version-1.0.0-0D1117?style=for-the-badge&labelColor=0D1117&color=A78BFA" alt="Version"/>
<img src="https://img.shields.io/badge/Windows-0D1117?style=for-the-badge&logo=windows&logoColor=A78BFA" alt="Windows"/>
<img src="https://img.shields.io/badge/Rust-0D1117?style=for-the-badge&logo=rust&logoColor=A78BFA" alt="Rust"/>
<img src="https://img.shields.io/badge/Discord%20RPC-0D1117?style=for-the-badge&logo=discord&logoColor=5865F2" alt="Discord"/>
<img src="https://img.shields.io/badge/license-MIT-0D1117?style=for-the-badge&labelColor=0D1117&color=22C55E" alt="License"/>

<br/><br/>

<a href="https://github.com/zMrKenn/youtubediscordrichpresence/releases/latest"><img src="https://img.shields.io/badge/Download%20the%20app-.exe-0D1117?style=for-the-badge&logo=github&logoColor=A78BFA&labelColor=0D1117&color=A78BFA" alt="Download the app"/></a>
<a href="https://chromewebstore.google.com/detail/dmilhomifdpkfmeelmcnkdphhclcjagn"><img src="https://img.shields.io/badge/Chrome%20Web%20Store-in%20review-0D1117?style=for-the-badge&logo=googlechrome&logoColor=A78BFA&labelColor=0D1117&color=F59E0B" alt="Chrome Web Store"/></a>

</div>

---

## ▸ what it is

Two small pieces that put the video or song you're playing on YouTube straight onto your
Discord profile — title, channel, a live progress bar, the thumbnail, and buttons.

```text
◉  browser extension  -  reads the real player (accurate on Music, playlists, radio)
◉  native tray app    -  Rust + egui, talks to Discord's local pipe (no bot, no tokens)
◉  works on           -  youtube.com  and  music.youtube.com
◉  ships as           -  one self-contained .exe, no runtime, no console
```

<div align="center"><sub>Not affiliated with YouTube, Google, or Discord.</sub></div>

---

## ▸ how it works

```text
 browser extension  ──HTTP──▶  youtube-rpc.exe  ──named pipe──▶  Discord desktop
 (reads the player)  :41414     (tray + window)      IPC             (your profile)
```

The extension pulls the current video from YouTube's own player and POSTs it to
`127.0.0.1`. The app picks it up and speaks Discord's local IPC directly. Everything
stays on your machine — no accounts, no trackers, nothing leaves `localhost`.

---

## ▸ install

**1. Get the browser extension**

- Chrome Web Store: **[install here](https://chromewebstore.google.com/detail/dmilhomifdpkfmeelmcnkdphhclcjagn)** — *currently in review, coming soon*
- or load it unpacked: `chrome://extensions` → Developer mode → **Load unpacked** → pick `./extension`

**2. Get the desktop app**

- **[Download youtube-rpc.exe](https://github.com/zMrKenn/youtubediscordrichpresence/releases/latest/download/youtube-rpc.exe)** and run it — a red play icon lands in your tray
- or build it from source: `cd app && cargo build --release`

Then play something on YouTube. Right-click the tray icon → **Start with Windows** to launch
it at login.

---

## ▸ features

```text
◉  accurate title / channel / album  straight from the player API
◉  live progress bar + real thumbnail
◉  Listening / Watching / Playing     switchable from the window
◉  closes to tray, starts silently at login, single-instance
◉  every string obfuscated in the binary  (obfstr)
◉  the Discord app id is baked in         no config file needed
```

---

## ▸ stack

<p align="center">
  <img src="https://skillicons.dev/icons?i=rust,js,discord&theme=dark" alt="Stack" />
</p>

---

## ▸ notes

```text
◉  buttons show to other people, not on your own profile   (same as Spotify)
◉  presence auto-clears ~30s after you close the tab
◉  reconnects on its own if you restart Discord
```

<div align="center">
  <br/>
  <sub>by <a href="https://keen.pub">Keen</a> · always easy · always fast</sub>
</div>
