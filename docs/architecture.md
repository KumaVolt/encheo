# Encheo architecture

A media player for Windows, Linux, macOS and Android (phones, tablets and Android TV). It plays
files from local disks, network shares, media servers and cloud drives, and turns them into a
library with artwork. See [parity.md](parity.md) for features.

## Shape

All Rust, one workspace.

```
┌──────────────────────────────────────────────────────────┐
│ Slint UI                                                  │  home, library, details, settings,
│   one codebase: mouse/keyboard, touch, TV remote (D-pad)  │  player controls
├───────────────────────┬──────────────────────────────────┤
│ Rust core             │ libmpv  (playback engine, via FFI)│
│   library + SQLite    │   FFmpeg decoders, hw decode,     │
│   sources (VFS)  ─────┼─▶ libass subtitles, HDR/tone map, │
│   servers, metadata   │   audio passthrough, A/V sync     │
│   sync, Trakt         │                                   │
└───────────────────────┴──────────────────────────────────┘
```

- **libmpv does all playback.** Codecs, subtitles, HDR, frame pacing, passthrough. We configure it;
  we don't write a player.
- **The Rust core does everything that makes it feel like a library:** indexing, filename matching,
  metadata, sources, media servers, sync. It has no UI and no playback code, so it's testable on its own.
- **Slint for the UI.** It's the Rust toolkit that runs on Windows, Linux, macOS and Android, has
  keyboard/D-pad focus navigation, and its free licence is GPL-3, which matches ours. egui is weak on
  Android TV; iced and gpui don't target Android.

## Playback with libmpv

We drive libmpv through its C API (`libmpv2-sys` bindings or our own thin `mpv` crate) and build libmpv
ourselves for each platform. Android reuses mpv-android's build scripts.

| Platform | Decode | Video output |
|---|---|---|
| Android | `hwdec=mediacodec` | `vo=mediacodec_embed` into a SurfaceView behind Slint's window: zero-copy, and HDR passthrough to the TV works. Needs a small Java/JNI shim in our Activity |
| macOS | `hwdec=videotoolbox` | mpv render API drawing into Slint's OpenGL context as an underlay (SDR). → native view for HDR/EDR |
| Windows | `hwdec=d3d11va` | Render API underlay (SDR). → native child window with `vo=gpu-next` + `d3d11` for HDR |
| Linux | `hwdec=vaapi` / `nvdec` | Render API underlay (OpenGL) |

Slint's `set_rendering_notifier` hands us its GL context before each frame. mpv draws the video there
and Slint draws the controls on top.

**HDR on desktop is the known weak spot of the underlay path** (mpv tone-maps to SDR). Phase 2 adds
a native-window mode for full-screen playback on macOS and Windows; until then HDR files play
correctly, just tone-mapped.

**Biggest risk:** the Android SurfaceView-behind-Slint setup. It's the first thing the spike proves.
Fallback is `hwdec=mediacodec-copy` into the GL underlay, which is fine for 1080p and marginal for 4K
on cheap TV boxes.

### Reading from our sources

mpv's own SMB support needs libsmbclient, which isn't available on Android and is awkward to bundle elsewhere, so the Rust VFS
registers a protocol with `mpv_stream_cb_add_ro` (for example `encheo://<source>/<path>`) and serves
reads and seeks from its own clients and range cache. Plex/Jellyfin/Emby and cloud drives hand mpv
plain HTTPS URLs.

### mpv settings we own

Per-device profiles kept in the Rust core: hardware decode on/off, audio passthrough codecs
(`audio-spdif=ac3,eac3,dts,dts-hd,truehd`), frame-rate matching (Android: `Surface.setFrameRate`;
desktop: display mode switch), subtitle style and fonts, tone mapping, speed with pitch kept
(`scaletempo2`), audio/subtitle delays remembered per show.

## Crates

```
crates/
  library        model, SQLite, indexing, filename parser, watch state
  vfs            local, smb, nfs, webdav, sftp, dlna, http + range cache; mpv stream callbacks
  servers        Plex, Jellyfin, Emby (browse, play URLs, markers, scrobble)
  cloud          Google Drive, Dropbox, OneDrive
  meta           TMDB/TVDB, NFO, embedded tags, artwork cache
  sync           optional sync: settings, history, favourites, end-to-end encrypted source credentials
  mpv            safe wrapper over libmpv: properties, events, render API, stream callbacks
  ui             Slint components and screens
apps/
  encheo          desktop binary
  encheo-android  Android app (android-activity, Gradle wrapper for the Activity and SurfaceView shim)
build/
  libmpv         per-platform libmpv/FFmpeg build scripts
```

## Licensing

Encheo is **GPL-3.0-or-later**, so GPL builds of mpv and FFmpeg are fine; we bundle them and
publish the build scripts. No closed-source dependencies.

- **Name and look:** original name, icons and artwork; no other product's branding.
- **TMDB:** free for non-commercial use with attribution. If a paid hosted sync service ever
  exists, check TMDB's commercial terms first.
- **Dolby Vision:** mpv plays profile 8 as HDR10 and tone-maps profile 5. Real DV only via
  passthrough on licensed Android TV devices.
- **Audio:** passthrough needs no licence from us; FFmpeg's DTS/TrueHD software decoding is patent
  territory in some countries. Common in open-source players, but noted.
- **Plex, Trakt, OpenSubtitles:** each needs a registered app/API key. Keys for official builds
  live in CI secrets; people building from source bring their own.
- **Distribution:** GitHub Releases (Windows installer, macOS notarised DMG, Linux AppImage/Flatpak,
  Android APK), then Google Play and F-Droid.

## Phases and size

1. **Spike (1–2 weeks):** Slint + libmpv on all four platforms playing a 4K HEVC HDR MKV from
   local disk and from SMB through the Rust stream callback. On an Android TV box: hardware decode,
   passthrough, frame-rate switch, D-pad around a player overlay. Go/no-go on the stack.
2. **Player (3–4 weeks):** sources (local, SMB, WebDAV), folder browsing, track and subtitle menus,
   resume, OpenSubtitles. First public beta.
3. **Library (4–6 weeks):** indexing, filename parser, TMDB, artwork home screen, Up Next, TV UI.
   This is the part that makes it feel polished.
4. **Servers + Trakt (3–4 weeks):** Jellyfin, Plex, Emby, skip intro/credits, scrobbling.
5. **Sync + release (2–3 weeks):** optional sync, packaging and signing for all four platforms. 1.0.
6. **Later:** desktop HDR native window, cloud drives, downloads, NFS/SFTP/DLNA, Chromecast,
   ambient mode, profiles.

Roughly 4–5 months to 1.0. libmpv takes away the hardest part (our own playback engine would add
months); building libmpv per platform and a TV-grade UI in Slint cost about 3–4 weeks more than
Flutter + media_kit would. Platform work (four packages, signing, store reviews) is still about a quarter of the time.
