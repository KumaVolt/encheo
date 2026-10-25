# Feature map

This map lists the features a polished media player is expected to have, on **Windows, Linux,
macOS and Android** (phones and Android TV included), and where Encheo stands.

✓ built · ◐ partly · → next · ✗ left out on purpose · ? decide

## Sources

| Feature | Status | Notes |
|---|---|---|
| Local files and folders | → | Phase 2 |
| SMB, WebDAV | → | Phase 2. Rust clients feeding mpv through stream callbacks |
| NFS, SFTP/FTP, UPnP/DLNA | → | Later |
| Server discovery on the local network, connection speed test | → | mDNS/NetBIOS |
| Jellyfin, Emby | → | Phase 4. Jellyfin first: open API, the users most likely to switch |
| Plex (PIN login, managed users, watchlist) | → | Phase 4. Direct play first, server transcoding as fallback |
| Dropbox, OneDrive | → | Later. Need registered OAuth apps |
| Google Drive | → | Browsing a user's whole Drive needs a "restricted" scope, and Google requires a paid security assessment before a public app may use it. Plan for that, or ship it as bring-your-own-API-key |
| Other cloud drives (Box, pCloud, Mega, Yandex…) | ? | On demand |
| Bookmarks and deep links into folders | → | |

## Library and metadata

| Feature | Status | Notes |
|---|---|---|
| Filename → movie/episode matching | → | The core of the "it just looks nice" feel. Needs a big filename test corpus |
| TMDB metadata, artwork, collections, cast and crew | → | Free for non-commercial use; a paid hosted service would need TMDB's commercial terms |
| Fix match, reload metadata, per-folder language | → | |
| Embedded tags (MKV/MP4), Kodi `.nfo` files | → | NFO helps Kodi switchers |
| Ratings (IMDb, Rotten Tomatoes) | → | Through Trakt or OMDb |
| Quick and full indexing, incremental refresh | → | |
| Reorderable home sections, favourites, collections, playlists | → | |
| Up Next, watched state, resume points | → | |
| Extras (trailers, featurettes) | → | |
| Search across every source | → | |
| Downloads for offline playback | → | Matters most on Android phones |

## Playback

All decoding and rendering is libmpv, so format coverage is FFmpeg's.

| Feature | Status | Notes |
|---|---|---|
| Containers: MKV, MP4/MOV, TS/M2TS, AVI, WMV/ASF, FLV, OGM, WebM, VOB, MXF, RMVB | → | Through mpv |
| Video: H.264, HEVC, AV1, VP9, MPEG-2, VC-1, MPEG-4 Part 2 | → | Hardware decode per OS; required for 4K on Android |
| Audio: AAC, AC-3, E-AC-3, DTS, TrueHD, FLAC, Opus, MP3, PCM | → | Decode, or passthrough to a receiver |
| Disc images (ISO, BDMV, VIDEO_TS) | → | Main title. ✗ Disc menus |
| HDR10, HLG; Dolby Vision as HDR10 / tone-mapped | ◐ | Passthrough on Android TV; desktop tone-maps until the native-window mode |
| Subtitles: SRT, ASS/SSA, PGS, VobSub, SMI | → | Rendered by mpv (libass) |
| Subtitle font picker, user fonts, style settings | → | |
| OpenSubtitles search and download | → | Needs an API key |
| Simplified ↔ traditional Chinese subtitle conversion | → | OpenCC data |
| Skip intro / credits from server markers, auto-skip | → | |
| Speed control without pitch change | → | mpv `scaletempo2` |
| Audio/subtitle delay; track choices remembered per show | → | |
| Frame-rate and dynamic-range matching | → | Android TV first |
| Picture in Picture | → | Android and desktop |
| Upscaling, ambient glow around the video | → | mpv shaders |
| Google Cast | → | Chromecasts can't play MKV, TrueHD or DTS, so the sender has to remux or transcode on the fly. Desktop speaks the Cast protocol directly; Android's official SDK needs Google Play services, so not in the F-Droid build |
| Play to smart TVs over DLNA | ? | Open protocol, many TVs support it; same remux problem as Cast |
| AirPlay | ✗ | Apple TVs only accept it from Apple devices without a fragile reverse-engineered client. Revisit later |
| Keyboard shortcuts, remappable | → | |

## App

| Feature | Status | Notes |
|---|---|---|
| Profiles with passcode | → | |
| Trakt scrobbling and sync | → | |
| Sync of settings, history, favourites, home layout and sources across devices | → | Optional; source passwords end-to-end encrypted |
| Onboarding | → | |
| Analytics | ✗ | None. Opt-in crash reports only |
