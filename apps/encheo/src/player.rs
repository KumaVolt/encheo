//! Playback through libmpv, drawn into Slint's OpenGL context underneath the UI.
//!
//! Slint calls our rendering notifier on its own thread (the UI thread) around every frame. On
//! setup we create mpv's render context against Slint's GL; before each frame mpv draws the video
//! into the framebuffer Slint is about to draw on, and the (transparent) player screen goes on top.

use libmpv2::Mpv;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use std::cell::RefCell;
use std::ffi::{CString, c_void};

/// Slint's `get_proc_address` closure, valid only while the setup callback runs. mpv resolves all
/// GL functions inside `create_render_context`, which we call during that window, so a raw pointer
/// to it is enough.
type ProcLoader<'a> = &'a dyn Fn(&std::ffi::CStr) -> *const c_void;
struct LoaderPtr(*const ProcLoader<'static>);

fn get_proc_address(ctx: &LoaderPtr, name: &str) -> *mut c_void {
    let Ok(name) = CString::new(name) else { return std::ptr::null_mut() };
    // SAFETY: only called by mpv from inside `create_render_context`, while the loader is alive.
    unsafe { (*ctx.0)(&name) as *mut c_void }
}

const OPTIONS: &[(&str, &str)] = &[
    ("vo", "libmpv"),
    ("hwdec", "auto-safe"),
    ("keep-open", "yes"),
    ("input-default-bindings", "no"),
    ("input-vo-keyboard", "no"),
    ("osc", "no"),
    ("ytdl", "no"),
    ("cache", "yes"),
    ("demuxer-max-bytes", "400MiB"),
    ("demuxer-readahead-secs", "60"),
    ("audio-client-name", "Encheo"),
    ("sub-auto", "fuzzy"),
    // Album covers embedded in music files would otherwise count as a video track.
    ("audio-display", "no"),
    ("alang", "en,eng"),
    ("slang", "en,eng"),
    ("sub-font-size", "46"),
    ("sub-border-size", "2.5"),
    ("sub-shadow-offset", "1"),
    ("tone-mapping", "auto"),
    ("target-colorspace-hint", "yes"),
];

pub struct Player {
    mpv: &'static Mpv,
    render: RefCell<Option<RenderContext<'static>>>,
    gl: RefCell<Option<glow::Context>>,
}

impl Player {
    pub fn new() -> Result<Self, libmpv2::Error> {
        let mpv = Mpv::with_initializer(|init| {
            // Builds differ in what they support (the Android libmpv lacks some options), so an
            // unknown option is logged and skipped rather than stopping the player.
            for (name, value) in OPTIONS {
                if let Err(e) = init.set_option(name, *value) {
                    eprintln!("mpv option {name}={value}: {e}");
                }
            }
            Ok(())
        })?;
        // One player for the life of the app; leaking it lets the render context borrow it for
        // 'static and live in the UI thread's state.
        Ok(Self { mpv: Box::leak(Box::new(mpv)), render: RefCell::new(None), gl: RefCell::new(None) })
    }

    /// Hook into Slint's GL. `on_frame` is called (from mpv's threads) whenever a new video frame is
    /// ready and the window should repaint.
    pub fn setup_gl(&self, loader: ProcLoader<'_>, on_frame: impl Fn() + Send + 'static) {
        // SAFETY: `loader` outlives this function; mpv and glow only call it before we return.
        let static_loader: ProcLoader<'static> = unsafe { std::mem::transmute(loader) };
        let ptr = LoaderPtr(&static_loader as *const _);
        let params = [
            RenderParam::ApiType(RenderParamApiType::OpenGl),
            RenderParam::InitParams(OpenGLInitParams { get_proc_address, ctx: ptr }),
        ];
        match self.mpv.create_render_context(params) {
            Ok(mut ctx) => {
                ctx.set_update_callback(on_frame);
                *self.render.borrow_mut() = Some(ctx);
            }
            Err(e) => eprintln!("mpv render context: {e}"),
        }
        let gl = unsafe { glow::Context::from_loader_function_cstr(|s| loader(s)) };
        *self.gl.borrow_mut() = Some(gl);
    }

    /// Draw the current video frame into the framebuffer Slint is rendering to.
    pub fn draw(&self, width: u32, height: u32) {
        let render = self.render.borrow();
        let gl_ref = self.gl.borrow();
        let (Some(ctx), Some(gl)) = (render.as_ref(), gl_ref.as_ref()) else {
            return;
        };
        // mpv changes GL state freely. Slint's Skia renderer (Android) caches GL state and
        // crashes on surprises (e.g. a vertex attribute left enabled), so put back exactly what
        // was there.
        // SAFETY: same thread, the UI's context is current during BeforeRendering.
        let saved = unsafe { GlState::save(gl) };
        if let Err(e) = ctx.render::<LoaderPtr>(saved.framebuffer, width as i32, height as i32, true) {
            eprintln!("mpv render: {e}");
        }
        unsafe { saved.restore(gl) };
    }

    pub fn teardown(&self) {
        *self.render.borrow_mut() = None;
        *self.gl.borrow_mut() = None;
    }

    pub fn load(&self, url: &str, start_secs: Option<f64>) {
        let start = start_secs.filter(|s| *s > 5.0).map(|s| format!("+{s:.0}")).unwrap_or_else(|| "none".into());
        let _ = self.mpv.set_property("start", start);
        let _ = self.mpv.set_property("pause", false);
        if let Err(e) = self.mpv.command("loadfile", &[url, "replace"]) {
            eprintln!("loadfile: {e}");
        }
    }

    /// Add to the end of the playlist (music queues).
    pub fn append(&self, url: &str) {
        let _ = self.mpv.command("loadfile", &[url, "append"]);
    }

    pub fn set_playlist_pos(&self, index: usize) {
        let _ = self.mpv.set_property("playlist-pos", index as i64);
    }

    pub fn playlist_pos(&self) -> Option<usize> {
        self.mpv.get_property::<i64>("playlist-pos").ok().and_then(|i| usize::try_from(i).ok())
    }

    pub fn next(&self) {
        let _ = self.mpv.command("playlist-next", &[]);
    }

    pub fn prev(&self) {
        let _ = self.mpv.command("playlist-prev", &[]);
    }

    pub fn stop(&self) {
        let _ = self.mpv.command("stop", &[]);
    }

    pub fn toggle_pause(&self) {
        let _ = self.mpv.command("cycle", &["pause"]);
    }

    pub fn seek_to(&self, secs: f64) {
        let _ = self.mpv.command("seek", &[&format!("{secs:.2}"), "absolute"]);
    }

    pub fn seek_by(&self, secs: f64) {
        let _ = self.mpv.command("seek", &[&format!("{secs:.2}"), "relative"]);
    }

    pub fn status(&self) -> Status {
        let mpv = self.mpv;
        Status {
            position: mpv.get_property::<f64>("time-pos").unwrap_or(0.0),
            duration: mpv.get_property::<f64>("duration").unwrap_or(0.0),
            paused: mpv.get_property::<bool>("pause").unwrap_or(false),
            buffering: mpv.get_property::<bool>("paused-for-cache").unwrap_or(false)
                || mpv.get_property::<bool>("idle-active").unwrap_or(false)
                || mpv.get_property::<f64>("time-pos").is_err(),
            ended: mpv.get_property::<bool>("eof-reached").unwrap_or(false),
        }
    }
}

/// An audio or subtitle track of the current file, as mpv numbers it.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: i64,
    pub label: String,
    pub selected: bool,
    /// Stream index inside the file (matches Plex's `Stream.index`); `None` for added files.
    pub ff_index: Option<i64>,
    /// Title mpv has for it; for subtitles we add ourselves this is our own tag.
    pub title: Option<String>,
    /// Language tag in the file, if any.
    pub lang: Option<String>,
    /// Short codec name, e.g. "SRT", "ASS", "EAC3".
    pub codec: Option<String>,
    pub forced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start: f64,
}

impl Player {
    fn prop_str(&self, name: &str) -> Option<String> {
        self.mpv.get_property::<String>(name).ok().filter(|s| !s.is_empty())
    }

    /// Audio (`"audio"`) or subtitle (`"sub"`) tracks of the current file.
    pub fn tracks(&self, kind: &str) -> Vec<Track> {
        let count = self.mpv.get_property::<i64>("track-list/count").unwrap_or(0);
        (0..count)
            .filter(|i| self.prop_str(&format!("track-list/{i}/type")).as_deref() == Some(kind))
            .map(|i| {
                let p = |k: &str| self.prop_str(&format!("track-list/{i}/{k}"));
                let info = TrackInfo {
                    lang: p("lang"),
                    title: p("title"),
                    codec: p("codec"),
                    channels: self.mpv.get_property::<i64>(&format!("track-list/{i}/demux-channel-count")).ok(),
                    forced: self.mpv.get_property::<bool>(&format!("track-list/{i}/forced")).unwrap_or(false),
                    external: self.mpv.get_property::<bool>(&format!("track-list/{i}/external")).unwrap_or(false),
                };
                Track {
                    id: self.mpv.get_property::<i64>(&format!("track-list/{i}/id")).unwrap_or(0),
                    label: info.label(kind),
                    selected: self.mpv.get_property::<bool>(&format!("track-list/{i}/selected")).unwrap_or(false),
                    ff_index: self.mpv.get_property::<i64>(&format!("track-list/{i}/ff-index")).ok().filter(|i| *i >= 0),
                    title: info.title.clone(),
                    lang: info.lang.clone().filter(|l| l != "und"),
                    codec: info.codec.as_deref().map(codec_name),
                    forced: info.forced,
                }
            })
            .collect()
    }

    /// Pick an audio or subtitle track; `None` turns subtitles off.
    pub fn select_track(&self, kind: &str, id: Option<i64>) {
        let prop = if kind == "audio" { "aid" } else { "sid" };
        let _ = match id {
            Some(id) => self.mpv.set_property(prop, id),
            None => self.mpv.set_property(prop, "no"),
        };
    }

    /// Load an extra subtitle file without switching to it.
    pub fn add_subtitle(&self, url: &str, title: &str) {
        let _ = self.mpv.command("sub-add", &[url, "auto", title]);
    }

    pub fn chapters(&self) -> Vec<Chapter> {
        let count = self.mpv.get_property::<i64>("chapter-list/count").unwrap_or(0);
        (0..count)
            .map(|i| Chapter {
                title: self.prop_str(&format!("chapter-list/{i}/title")).unwrap_or_else(|| format!("Chapter {}", i + 1)),
                start: self.mpv.get_property::<f64>(&format!("chapter-list/{i}/time")).unwrap_or(0.0),
            })
            .collect()
    }

    pub fn current_chapter(&self) -> Option<usize> {
        self.mpv.get_property::<i64>("chapter").ok().and_then(|c| usize::try_from(c).ok())
    }

    pub fn step_chapter(&self, delta: i64) {
        let _ = self.mpv.command("add", &["chapter", &delta.to_string()]);
    }

    /// 0.0–1.0 (mpv's 0–100).
    pub fn volume(&self) -> f64 {
        self.mpv.get_property::<f64>("volume").unwrap_or(100.0) / 100.0
    }

    pub fn set_volume(&self, v: f64) {
        let _ = self.mpv.set_property("volume", (v.clamp(0.0, 1.0) * 100.0).round());
        let _ = self.mpv.set_property("mute", false);
    }

    pub fn muted(&self) -> bool {
        self.mpv.get_property::<bool>("mute").unwrap_or(false)
    }

    pub fn toggle_mute(&self) {
        let _ = self.mpv.command("cycle", &["mute"]);
    }

    pub fn speed(&self) -> f64 {
        self.mpv.get_property::<f64>("speed").unwrap_or(1.0)
    }

    pub fn set_speed(&self, speed: f64) {
        let _ = self.mpv.set_property("speed", speed);
    }

    /// Seek while dragging the scrubber: snaps to keyframes, so it keeps up.
    pub fn seek_fast(&self, secs: f64) {
        let _ = self.mpv.command("seek", &[&format!("{secs:.2}"), "absolute+keyframes"]);
    }
}

struct TrackInfo {
    lang: Option<String>,
    title: Option<String>,
    codec: Option<String>,
    channels: Option<i64>,
    forced: bool,
    external: bool,
}

impl TrackInfo {
    /// "English · EAC3 5.1", "Dutch (Forced) · PGS", "Commentary · AAC Stereo".
    fn label(&self, kind: &str) -> String {
        let mut name = self.lang.as_deref().map(language_name).unwrap_or_else(|| "Unknown".into());
        if let Some(t) = &self.title
            && !t.eq_ignore_ascii_case(&name)
        {
            name = if self.lang.is_some() { format!("{name} – {t}") } else { t.clone() };
        }
        if self.forced {
            name.push_str(" (Forced)");
        }
        if self.external {
            name.push_str(" (External)");
        }
        let codec = self.codec.as_deref().map(codec_name);
        let detail = match (kind, codec, self.channels) {
            ("audio", Some(c), Some(n)) => format!("{c} {}", channel_label(n)),
            (_, Some(c), _) => c,
            _ => return name,
        };
        format!("{name} · {detail}")
    }
}

fn language_name(code: &str) -> String {
    match code.to_ascii_lowercase().as_str() {
        "en" | "eng" => "English",
        "nl" | "nld" | "dut" => "Dutch",
        "fr" | "fra" | "fre" => "French",
        "de" | "deu" | "ger" => "German",
        "es" | "spa" => "Spanish",
        "it" | "ita" => "Italian",
        "pt" | "por" => "Portuguese",
        "ja" | "jpn" => "Japanese",
        "ko" | "kor" => "Korean",
        "zh" | "zho" | "chi" => "Chinese",
        "ru" | "rus" => "Russian",
        "ar" | "ara" => "Arabic",
        "hi" | "hin" => "Hindi",
        "sv" | "swe" => "Swedish",
        "da" | "dan" => "Danish",
        "no" | "nor" | "nob" => "Norwegian",
        "fi" | "fin" => "Finnish",
        "pl" | "pol" => "Polish",
        "tr" | "tur" => "Turkish",
        "und" => "Unknown",
        other => return other.to_uppercase(),
    }
    .into()
}

fn codec_name(codec: &str) -> String {
    match codec {
        "subrip" => "SRT".into(),
        "hdmv_pgs_subtitle" => "PGS".into(),
        "dvd_subtitle" => "VobSub".into(),
        "ass" => "ASS".into(),
        "mov_text" => "Text".into(),
        "webvtt" => "WebVTT".into(),
        "truehd" => "TrueHD".into(),
        "dts" => "DTS".into(),
        "eac3" => "EAC3".into(),
        "ac3" => "AC3".into(),
        "aac" => "AAC".into(),
        "flac" => "FLAC".into(),
        "opus" => "Opus".into(),
        other => other.to_uppercase(),
    }
}

fn channel_label(n: i64) -> String {
    match n {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n}ch"),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub buffering: bool,
    pub ended: bool,
}

/// `1:02:03` / `42:07`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_formats() {
        assert_eq!(super::clock(0.0), "0:00");
        assert_eq!(super::clock(65.4), "1:05");
        assert_eq!(super::clock(3723.0), "1:02:03");
        assert_eq!(super::clock(-3.0), "0:00");
    }

    #[test]
    fn track_labels() {
        let t = |lang: Option<&str>, title: Option<&str>, codec: &str, ch: Option<i64>, forced: bool| super::TrackInfo {
            lang: lang.map(Into::into),
            title: title.map(Into::into),
            codec: Some(codec.into()),
            channels: ch,
            forced,
            external: false,
        };
        assert_eq!(t(Some("eng"), None, "eac3", Some(6), false).label("audio"), "English · EAC3 5.1");
        assert_eq!(t(Some("dut"), None, "hdmv_pgs_subtitle", None, true).label("sub"), "Dutch (Forced) · PGS");
        assert_eq!(t(Some("eng"), Some("Commentary"), "aac", Some(2), false).label("audio"), "English – Commentary · AAC Stereo");
        assert_eq!(t(None, Some("Signs & Songs"), "ass", None, false).label("sub"), "Signs & Songs · ASS");
    }
}

/// The GL state a UI renderer relies on, captured before mpv draws and restored after.
struct GlState {
    framebuffer: i32,
    vertex_array: i32,
    array_buffer: i32,
    program: i32,
    active_texture: i32,
    textures: [i32; 4],
    viewport: [i32; 4],
    scissor_box: [i32; 4],
    scissor: bool,
    blend: bool,
    depth_test: bool,
    stencil_test: bool,
    cull_face: bool,
    blend_func: [i32; 4],
    blend_eq: [i32; 2],
    unpack_alignment: i32,
    attribs: [bool; 16],
}

impl GlState {
    unsafe fn save(gl: &glow::Context) -> Self {
        use glow::HasContext;
        unsafe {
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let mut scissor_box = [0; 4];
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor_box);
            let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
            let mut textures = [0; 4];
            for (i, t) in textures.iter_mut().enumerate() {
                gl.active_texture(glow::TEXTURE0 + i as u32);
                *t = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);
            }
            gl.active_texture(active_texture as u32);
            let mut attribs = [false; 16];
            for (i, a) in attribs.iter_mut().enumerate() {
                let mut v = [0.0f32; 4];
                gl.get_vertex_attrib_parameter_f32_slice(i as u32, glow::VERTEX_ATTRIB_ARRAY_ENABLED, &mut v);
                *a = v[0] != 0.0;
            }
            GlState {
                framebuffer: gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING),
                vertex_array: gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING),
                array_buffer: gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING),
                program: gl.get_parameter_i32(glow::CURRENT_PROGRAM),
                active_texture,
                textures,
                viewport,
                scissor_box,
                scissor: gl.is_enabled(glow::SCISSOR_TEST),
                blend: gl.is_enabled(glow::BLEND),
                depth_test: gl.is_enabled(glow::DEPTH_TEST),
                stencil_test: gl.is_enabled(glow::STENCIL_TEST),
                cull_face: gl.is_enabled(glow::CULL_FACE),
                blend_func: [
                    gl.get_parameter_i32(glow::BLEND_SRC_RGB),
                    gl.get_parameter_i32(glow::BLEND_DST_RGB),
                    gl.get_parameter_i32(glow::BLEND_SRC_ALPHA),
                    gl.get_parameter_i32(glow::BLEND_DST_ALPHA),
                ],
                blend_eq: [gl.get_parameter_i32(glow::BLEND_EQUATION_RGB), gl.get_parameter_i32(glow::BLEND_EQUATION_ALPHA)],
                unpack_alignment: gl.get_parameter_i32(glow::UNPACK_ALIGNMENT),
                attribs,
            }
        }
    }

    unsafe fn restore(&self, gl: &glow::Context) {
        use glow::HasContext;
        use std::num::NonZeroU32;
        let id = |v: i32| NonZeroU32::new(v as u32);
        let set = |cap: u32, on: bool| unsafe { if on { gl.enable(cap) } else { gl.disable(cap) } };
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, id(self.framebuffer).map(glow::NativeFramebuffer));
            gl.bind_vertex_array(id(self.vertex_array).map(glow::NativeVertexArray));
            for (i, on) in self.attribs.iter().enumerate() {
                if *on {
                    gl.enable_vertex_attrib_array(i as u32);
                } else {
                    gl.disable_vertex_attrib_array(i as u32);
                }
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, id(self.array_buffer).map(glow::NativeBuffer));
            gl.use_program(id(self.program).map(glow::NativeProgram));
            for (i, t) in self.textures.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + i as u32);
                gl.bind_texture(glow::TEXTURE_2D, id(*t).map(glow::NativeTexture));
            }
            gl.active_texture(self.active_texture as u32);
            let [x, y, w, h] = self.viewport;
            gl.viewport(x, y, w, h);
            let [x, y, w, h] = self.scissor_box;
            gl.scissor(x, y, w, h);
            set(glow::SCISSOR_TEST, self.scissor);
            set(glow::BLEND, self.blend);
            set(glow::DEPTH_TEST, self.depth_test);
            set(glow::STENCIL_TEST, self.stencil_test);
            set(glow::CULL_FACE, self.cull_face);
            let [sr, dr, sa, da] = self.blend_func;
            gl.blend_func_separate(sr as u32, dr as u32, sa as u32, da as u32);
            gl.blend_equation_separate(self.blend_eq[0] as u32, self.blend_eq[1] as u32);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, self.unpack_alignment);
        }
    }
}
