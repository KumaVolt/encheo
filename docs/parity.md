# Feature map

This map lists the features a polished media player is expected to have, on **Windows, Linux,
macOS and Android** (phones and Android TV included), and where Encheo stands.

✓ built · ◐ partly · → next · ✗ left out on purpose · ? decide

## Sources

| Feature | Status | Notes |
|---|---|---|
| Plex (PIN sign-in, servers, libraries, watchlist) | ✓ | Direct play. Managed-user switching left out on purpose: the signed-in user is the viewer |
| Jellyfin, Emby | ✓ | One client for both; resume points, watched state, intro/credits (Jellyfin media segments) |
| Local folders | ✓ | Desktop. Android needs storage permission handling (→) |
| SMB, WebDAV | ✓ | Pure-Rust SMB2/3; WebDAV played directly over HTTP |
| SFTP, FTP/FTPS | ✓ | SFTP host key pinned on first connect |
| NFSv3 | ✓ | Tries a privileged port first, explains the NAS setting otherwise |
| DLNA/UPnP | ✓ | |
| Discovery on the local network | ✓ | Jellyfin/Emby (UDP 7359), Plex (GDM), SMB/WebDAV (mDNS), DLNA (SSDP). NetBIOS-only SMB servers aren't found (→) |
| Connection speed test | ✓ | Reads a real video from the source |
| Dropbox, OneDrive | → | Need registered OAuth apps |
| Google Drive | → | Browsing a whole Drive needs a "restricted" scope and Google's paid security assessment; or bring-your-own-API-key |
| Other cloud drives (Box, pCloud, Mega, Yandex…) | ? | On demand |
| Bookmarks and deep links into folders | → | |

## Library and metadata

| Feature | Status | Notes |
|---|---|---|
| Filename → movie/episode parsing | ◐ | Parser done (scene names, SxxEyy, 1x05, anime "Show - 07"); used for titles in file sources |
| TMDB metadata and artwork for file sources | → | Free for non-commercial use with attribution |
| Fix match, reload metadata, per-folder language | → | |
| Embedded tags (MKV/MP4), Kodi `.nfo` files | → | |
| Ratings (IMDb, Rotten Tomatoes, TMDB) | ✓ | From Plex and Jellyfin/Emby |
| Cast and crew, related titles | ✓ | |
| Home: Continue Watching, Recently Added per library, Watchlist | ✓ | Recently Added groups episodes into their show |
| Favourites | ✓ | Kept on the device; a row on the home screen |
| Reorderable home sections, collections, playlists | → | |
| Up Next, watched state, resume points | ✓ | Servers keep their own; files tracked locally |
| Extras (trailers, featurettes) | → | |
| Search | ✓ | Media servers and file sources (file lists collected on the first search) |
| Downloads for offline playback | → | Matters most on Android phones |

## Playback

All decoding and rendering is libmpv, so format coverage is FFmpeg's.

| Feature | Status | Notes |
|---|---|---|
| Containers and codecs (MKV, MP4, TS, AVI, WMV…; H.264, HEVC, AV1, VP9, VC-1…; AAC, AC-3, E-AC-3, DTS, TrueHD, FLAC…) | ✓ | Hardware decoding on each OS |
| Audio passthrough to a receiver | → | |
| Disc images (ISO, BDMV, VIDEO_TS) | → | Main title. ✗ Disc menus |
| HDR10, HLG; Dolby Vision as HDR10 / tone-mapped | ◐ | Detected and labelled; tone-mapped on desktop |
| Subtitles: SRT, ASS/SSA, PGS, VobSub | ✓ | Rendered by mpv (libass); external server subtitles loaded |
| Subtitle language detection for untagged tracks | ✓ | From the subtitle text, offline |
| Audio/subtitle choice remembered per show (by language) | ✓ | Also saved to the show's Plex settings |
| Subtitle size | ✓ | |
| Subtitle font picker, user fonts, colours | → | |
| Subtitle/audio delay | ✓ | 0.1 s steps |
| Online subtitle search | ◐ | Through Plex (OpenSubtitles), or Jellyfin/Emby with a subtitle plugin. Files from shares (→, needs our own key) |
| Simplified ↔ traditional Chinese subtitle conversion | → | |
| Skip intro / credits | ✓ | From server markers; optional auto-skip |
| Next episode autoplay, episode list, chapters | ✓ | |
| Speed control without pitch change | ✓ | |
| Frame-rate and dynamic-range matching | → | Android TV first |
| Picture in Picture | ◐ | Desktop. Android (→) |
| Upscaling, ambient glow around the video | → | mpv shaders |
| Google Cast | → | Chromecasts can't play MKV, TrueHD or DTS: needs remux/transcode on the fly |
| Play to smart TVs over DLNA | ? | Same remux problem as Cast |
| AirPlay | ✗ | Apple TVs only reliably accept it from Apple devices |
| Keyboard shortcuts | ✓ | Space, arrows, F, M, N, P, Esc. Remapping (→) |
| TV remote (D-pad) navigation | ✓ | |

## App

| Feature | Status | Notes |
|---|---|---|
| macOS, Windows, Linux, Android phone, Android TV | ◐ | All build in CI; macOS and Android tested; Windows and Linux untested |
| iPhone, iPad | ◐ | Experimental: unsigned .ipa for sideloading; video via mpv on Metal (MoltenVK), tested in the simulator only. App Store needs libsmbclient dropped and the LGPL relinking terms met |
| Apple TV | ✗ | Slint doesn't support tvOS |
| Profiles with passcode | → | |
| Trakt scrobbling and sync | → | |
| Sync of settings, history, favourites, home layout and sources across devices | → | Optional; source passwords end-to-end encrypted |
| Credentials in the OS keychain | → | Stored in a file only the user can read for now |
| Onboarding | → | |
| Analytics | ✗ | None. Opt-in crash reports only |
