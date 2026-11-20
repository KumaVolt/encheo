Preview of Encheo, an open-source media player for Windows, Linux, macOS, Android (phones and
Android TV) and, experimentally, iPhone and iPad. Early software: expect rough edges.

## New in 0.6.0

- **Requests through Seerr:** add your Overseerr or Jellyseerr server under Settings → Sources
  (blank user name signs in with your Plex account). Then request films, or shows season by
  season, from their page, and follow it there: waiting for approval, downloading with a progress
  bar and time left, then available. Without Plex, search finds titles you don't have through Seerr
- **Library filters:** sort films and shows by title, date added, release date or rating, and
  filter by watched, genre, country (Plex: Korean, Japanese and so on), decade and age rating.
  Kept per library. An A–Z rail on the side jumps through the grid
- **Smarter search:** "Did you mean" for typos, titles you don't own from Plex's catalogue (with a
  page, trailer and Add to Watchlist), and what you do own that's like them
- **Plex Home profiles:** a "Who's watching?" picker at start and in Settings, with a PIN pad for
  protected profiles. Works with the TV remote
- **HDR on Android (experimental, off by default):** HDR10/HLG output and surround passthrough.
  On iPhone and iPad, HDR video now shows as HDR
- **Trailers play inside the app,** also on Android
- **Home rows** load more titles as you scroll
- **More file types** in file sources (mxf, wtv, m4b, mka, dts, TrueHD, WavPack and others), and
  `.strm` files that point at a stream
- **Less memory:** large pictures are shrunk, and cast photos come through your server (browsing
  peaked at about 1.4 GB, now about 540 MB)
- **Licences:** third-party notices under About → Licences, and with every download

## In 0.5.0

- **Synced lyrics:** the microphone button in Now Playing shows the words, with the line being
  sung lit up as the song plays. Tap a line to jump there. Lyrics come from your Plex or
  Jellyfin server, else from LRCLIB (only the song's title, artist, album and length are sent)
- **Now Playing:** a full-screen music view with a large cover, a seek bar and Up Next. Open it
  from the music bar's cover or title
- **Music:** Artists, Albums, Songs and Playlists tabs; playlists play or shuffle
- **Collections** from Plex and Jellyfin get a row on Home and a page of their own
- **TV Home, Netflix style:** a full-screen banner shows the title the remote is on, with wide
  cards in the rows. On every device the banner folds away as you scroll, and Home has more rows
  (including Plex's own suggestions) to browse
- **Watched marks:** a tick on watched films and episodes, and on seasons you've finished
- **Anime trailers** from AniList when the server has none, and trailers that kept loading forever
  now play
- **Android:** music keeps playing with the screen off, with controls on the lock screen and in
  the notification. Built with Gradle now
- **New icon** on every platform
- **Less memory:** the picture cache has a size limit

## In 0.4.1

- **Windows installer:** `encheo-windows-x86_64-setup.exe` installs Encheo with a Start menu entry
  and an uninstaller. The zip is still there as a portable version
- **Windows:** no more black console window opening behind the app

## In 0.4.0

- **Featured title on Home:** a large banner with the artwork, summary and Play / More Info
- **Trailers:** a Trailer button on film and show pages when your Plex or Jellyfin/Emby server has
  one. Online-only Jellyfin trailers open in the browser or YouTube app
- **Settings screen:** sources, sign out, refresh and About now live in one place, behind the gear
  icon, so they are hard to press by accident. Edit or remove sources from there
- **TV mode:** on Android TV the side menu shrinks to icons, text is larger, and the selected
  poster grows with a white frame so it is easy to see from the sofa
- **Remote Back button:** goes back a step (Settings, search, library, top of Home, menu) instead
  of leaving the app

## In 0.3.0

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
| `encheo-windows-x86_64-setup.exe` | Windows 10/11, 64-bit | Installer: adds Encheo to the Start menu, no administrator rights needed. Not code-signed yet, so SmartScreen may warn: More info → Run anyway |
| `encheo-windows-x86_64.zip` | Windows 10/11, 64-bit | Portable: unzip the whole folder first, then run `encheo.exe` (it needs `libmpv-2.dll` next to it) |
| `encheo-linux-x86_64.tar.gz` | Linux, x86-64 | Needs libmpv: `sudo apt install libmpv2` (or your distro's package) |
| `encheo-ios.ipa` | iPhone and iPad, iOS 16+ (experimental) | Unsigned: install with AltStore or SideStore, which sign it with your Apple ID. Video playback is new and only tested in the simulator |
| `encheo-android-arm64.apk` | Android 8+ phones, tablets, Android TV (arm64) | Sideload. Signed with a temporary key: uninstall before installing a later preview |

Windows and Linux builds haven't been tested by hand yet; reports welcome.
