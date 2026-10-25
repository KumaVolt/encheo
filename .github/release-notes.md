Preview of Encheo, an open-source media player for Windows, Linux, macOS, Android (phones and
Android TV) and, experimentally, iPhone and iPad. Early software: expect rough edges.

## New in 0.3.0

- **iPhone and iPad (experimental):** the whole app, with video through mpv on Metal. Unsigned
  `.ipa` for sideloading with AltStore or SideStore. Only tested in the iOS simulator so far, where
  some 10-bit films crash the simulator's graphics driver; reports from real devices welcome.

## In 0.2.0

- Find subtitles online through your Plex/Jellyfin/Emby server; subtitle size; subtitle and audio
  delay; auto-skip intros and credits; search inside file sources; Favourites

## What's in it

- **Media servers:** Plex (sign in with a code, watchlist), Jellyfin and Emby
- **File sources:** local folders, SMB, WebDAV, SFTP, FTP/FTPS, NFS and DLNA, with network
  discovery and a speed test
- **Library:** Continue Watching, Recently Added, details with ratings, cast and related titles
- **Player** (libmpv): nearly every format, hardware decoding, skip intro/credits, next episode,
  chapters, speed, Picture in Picture (desktop), TV remote navigation
- **Subtitles:** language detection for untagged tracks, audio/subtitle language remembered per
  show, subtitles found online through your Plex/Jellyfin/Emby server
- **Music:** artists, albums, play queue

## Installing

| File | Platform | Notes |
|---|---|---|
| `encheo-macos-arm64.zip` | macOS, Apple silicon | Needs libmpv: `brew install mpv`. Not notarized yet: right-click the app → Open the first time |
| `encheo-windows-x86_64.zip` | Windows 10/11, 64-bit | Unzip and run `encheo.exe`. Not code-signed yet, so SmartScreen may warn |
| `encheo-linux-x86_64.tar.gz` | Linux, x86-64 | Needs libmpv: `sudo apt install libmpv2` (or your distro's package) |
| `encheo-ios.ipa` | iPhone and iPad, iOS 16+ (experimental) | Unsigned: install with AltStore or SideStore, which sign it with your Apple ID. Video playback is new and only tested in the simulator |
| `encheo-android-arm64.apk` | Android 8+ phones, tablets, Android TV (arm64) | Sideload. Signed with a temporary key: uninstall before installing a later preview |

Windows and Linux builds haven't been tested by hand yet; reports welcome.
