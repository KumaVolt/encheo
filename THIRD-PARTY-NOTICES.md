# Third-party notices

Encheo is licensed under the [Mozilla Public License 2.0](LICENSE). It is built on other people's
open-source work, listed here with their licences. The full licence texts are in
[`licenses/`](licenses/), and the licences of the Rust crates compiled into Encheo follow at the
end of the notices shown in the app (Settings → About → Licences) and of
`THIRD-PARTY-NOTICES.txt` in the downloads.

## Source code

Encheo's own source code, with the scripts that fetch and link everything below, is at
<https://github.com/KumaVolt/encheo>. Anyone can rebuild Encheo with a modified version of any of
these libraries; that is how Encheo meets the LGPL's relinking terms on platforms where the
libraries are linked statically (iOS).

On request we will provide the complete corresponding source code of every LGPL- and GPL-licensed
library in the version a given Encheo release ships, for at least three years after that release.
Ask by opening an issue at <https://github.com/KumaVolt/encheo/issues>.

## Playback: libmpv and FFmpeg

| Library | Licence | Home |
|---|---|---|
| mpv (libmpv), built without its GPL parts | LGPL 2.1 or later | <https://mpv.io> |
| FFmpeg | LGPL 3 or later (iOS, Android); see Windows below | <https://ffmpeg.org> |

FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.

## iOS and iPadOS

Linked statically from [MPVKit 1.0.0](https://github.com/mpvkit/MPVKit/releases/tag/1.0.0)'s
LGPL builds (not the `-GPL` ones), fetched by `scripts/ios-libs.sh`. MPVKit's build scripts for
that release list each library's exact version and source.

| Library | Version | Licence | Text |
|---|---|---|---|
| mpv | 0.41.0 | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| FFmpeg | 8.1.2 | LGPL 3 or later | [LGPL-3.0](licenses/LGPL-3.0.txt) |
| libplacebo | 7.360.1 | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| libbluray | 1.4.0 | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| GNU FriBidi | MPVKit libass-build 0.17.5 | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| GnuTLS | 3.8.11 | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| Nettle (with hogweed) | MPVKit gnutls-build 3.8.11 | LGPL 3 or later (or GPL 2 or later) | [LGPL-3.0](licenses/LGPL-3.0.txt) |
| GNU MP (gmp) | MPVKit gnutls-build 3.8.11 | LGPL 3 or later (or GPL 2 or later) | [LGPL-3.0](licenses/LGPL-3.0.txt) |
| uchardet | 0.0.8 | LGPL 2.1 or later (or MPL 1.1, or GPL 2 or later) | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| libass | MPVKit libass-build 0.17.5 | ISC | [libass](licenses/libass.txt) |
| FreeType | MPVKit libass-build 0.17.5 | FreeType License | [FTL](licenses/freetype-FTL.txt) |
| HarfBuzz | MPVKit libass-build 0.17.5 | "Old MIT" | [harfbuzz](licenses/harfbuzz.txt) |
| libunibreak | MPVKit libass-build 0.17.5 | zlib | [libunibreak](licenses/libunibreak.txt) |
| dav1d | 1.5.3 | BSD 2-Clause | [dav1d](licenses/dav1d.txt) |
| uavs3d | 1.2.1 | BSD 3-Clause | [uavs3d](licenses/uavs3d.txt) |
| Little CMS (lcms2) | 2.17.0 | MIT | [lcms2](licenses/lcms2.txt) |
| libdovi | 3.3.2 | MIT | [libdovi](licenses/libdovi.txt) |
| OpenSSL | 3.3.5 | Apache 2.0 | [Apache-2.0](licenses/Apache-2.0.txt) |
| MoltenVK and the Vulkan headers | MPVKit vulkan 1.4.2 | Apache 2.0 | [Apache-2.0](licenses/Apache-2.0.txt) |
| shaderc (with glslang and SPIRV-Tools) | 2025.5.0 | Apache 2.0 (glslang also BSD 3-Clause and others) | [Apache-2.0](licenses/Apache-2.0.txt) |

Portions of this software are copyright © The FreeType Project (www.freetype.org). All rights
reserved.

## Android

Linked from [media-kit/libmpv-android-video-build v1.1.11](https://github.com/media-kit/libmpv-android-video-build/releases/tag/v1.1.11)
("full" variant, built with `-Dgpl=false`), fetched by `scripts/android-libs.sh`. Its build
scripts list each library's exact version and source.

| Library | Licence | Text |
|---|---|---|
| mpv | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| FFmpeg | LGPL 3 or later | [LGPL-3.0](licenses/LGPL-3.0.txt) |
| GNU FriBidi | LGPL 2.1 or later | [LGPL-2.1](licenses/LGPL-2.1.txt) |
| libass | ISC | [libass](licenses/libass.txt) |
| FreeType | FreeType License | [FTL](licenses/freetype-FTL.txt) |
| HarfBuzz | "Old MIT" | [harfbuzz](licenses/harfbuzz.txt) |
| dav1d | BSD 2-Clause | [dav1d](licenses/dav1d.txt) |
| libxml2 | MIT | [libxml2](licenses/libxml2.txt) |
| Mbed TLS | Apache 2.0 | [Apache-2.0](licenses/Apache-2.0.txt) |

## Windows

The download includes `libmpv-2.dll` from
[shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake), loaded
dynamically. Those builds include GPL parts, so `libmpv-2.dll` is licensed under the
[GPL 3 or later](licenses/GPL-3.0.txt). The exact build is named in `libmpv-build.txt` next to it;
its source and build scripts are in that project's matching release. Encheo's own code stays under
the MPL 2.0, which allows being combined with GPL code this way.

## macOS and Linux

Encheo uses the libmpv installed on the system (Homebrew's `mpv` on macOS, the distribution's
package on Linux); it is not part of the download.

## User interface: Slint

Encheo's user interface is built with [Slint](https://slint.dev), used under the
[Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0](licenses/Slint-Royalty-free-2.0.md).
As that licence asks, About shows Slint's "About Slint" badge.

## Rust crates

Encheo is written in Rust and compiled together with several hundred crates from
[crates.io](https://crates.io), almost all under MIT, Apache 2.0, BSD, ISC, zlib or Unicode
licences. Two, `libmpv2` and `libmpv2-sys` (the bindings to libmpv), are under the LGPL 2.1.
`scripts/notices.sh` collects each crate's licence text with
[cargo-about](https://github.com/EmbarkStudios/cargo-about); the result is shown in the app and
shipped as `THIRD-PARTY-NOTICES.txt`.
