//! Android: HDR video and surround sound sent to the TV as they are, where the TV takes them.
//!
//! Normally mpv draws into Slint's OpenGL frame, which is SDR: HDR gets converted to normal
//! brightness and colour. For HDR10 and HLG the hardware decoder can instead draw straight onto a
//! video layer behind the UI (`EncheoActivity.java`), and the TV switches to HDR. Encheo then
//! draws the subtitles itself; picture-based subtitles (Blu-ray PGS, DVD) need mpv's drawing, so
//! picking one goes back to the converted picture.
//!
//! Experimental and off by default (Settings): not yet tried on a real HDR TV.
//!
//! What the TV and sound output take is checked once at startup (a few milliseconds) and kept for
//! the session. It isn't stored: the same box on another TV or receiver gives another answer.

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM, NativeMethod};
use libmpv2::Mpv;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What the screen and sound output accept untouched.
#[derive(Default, Debug)]
pub struct Caps {
    /// "hdr10", "hlg", "dv", "hdr10+".
    pub hdr: Vec<String>,
    /// mpv's passthrough names ("ac3", "eac3", "dts", "dts-hd", "truehd"), plus "atmos".
    pub audio: Vec<String>,
}

static CAPS: OnceLock<Caps> = OnceLock::new();
static MPV: OnceLock<&'static Mpv> = OnceLock::new();
static CLASS: OnceLock<GlobalRef> = OnceLock::new();
/// Testing on a screen without HDR: `adb shell setprop debug.encheo.direct 1` sends every
/// video down the direct path.
static FORCED: OnceLock<bool> = OnceLock::new();

/// The video layer and who's drawing on it. Changed from the UI thread and from Java's.
struct Layer {
    surface: Option<GlobalRef>,
    /// Encheo wants the direct path for the current video.
    want: bool,
    mode: Mode,
    /// mpv's own hardware decoding setting, put back after the direct path.
    hwdec: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// mpv draws into Slint's frame.
    Normal,
    /// The decoder draws onto the video layer.
    Direct,
    /// Direct, but the layer is gone for now (the app left the screen).
    Suspended,
}

/// Mirrors `Layer::mode != Normal`, for the renderer to read every frame without the lock.
static DIRECT: AtomicBool = AtomicBool::new(false);

static LAYER: Mutex<Layer> = Mutex::new(Layer { surface: None, want: false, mode: Mode::Normal, hwdec: String::new() });

/// The video being followed (UI thread only).
#[derive(Default)]
struct File {
    path: String,
    /// mpv's transfer function once known: "pq" (HDR10, Dolby Vision's HDR10 base), "hlg", others SDR.
    gamma: Option<String>,
    /// The direct path didn't work for this file (no hardware decoder for it).
    failed: bool,
    direct_since: Option<Instant>,
}

thread_local! {
    static FILE: RefCell<File> = RefCell::new(File::default());
}

/// Picture-based subtitle formats, which only mpv can draw.
const IMAGE_SUBS: &[&str] = &["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "xsub", "dvb_teletext"];

/// Startup: check the screen and sound output, and get ready to play onto the video layer.
pub fn init(mpv: &'static Mpv) -> &'static Caps {
    let _ = MPV.set(mpv);
    let _ = FORCED.set(crate::player::android_prop("debug.encheo.direct"));
    let caps = with_env(|env, activity, class| {
        let s = env
            .call_static_method(class, "capabilities", "(Landroid/app/Activity;)Ljava/lang/String;", &[JValue::Object(activity)])?
            .l()?;
        let s: String = env.get_string(&s.into())?.into();
        Ok(parse(&s))
    })
    .unwrap_or_default();
    eprintln!("display and audio: {caps:?}");
    CAPS.get_or_init(|| caps)
}

fn parse(s: &str) -> Caps {
    let mut caps = Caps::default();
    for part in s.split(';') {
        let Some((key, values)) = part.split_once('=') else { continue };
        let values = values.split(',').filter(|v| !v.is_empty()).map(String::from).collect();
        match key {
            "hdr" => caps.hdr = values,
            "audio" => caps.audio = values,
            _ => {}
        }
    }
    caps
}

/// mpv's `audio-spdif`: the formats to pass through (empty: decode everything).
pub fn spdif(enabled: bool) -> String {
    match CAPS.get() {
        Some(c) if enabled => c.audio.iter().filter(|a| *a != "atmos").cloned().collect::<Vec<_>>().join(","),
        _ => String::new(),
    }
}

/// Settings: what was found, in words.
pub fn summary() -> String {
    let Some(c) = CAPS.get() else { return String::new() };
    let hdr: Vec<&str> = c
        .hdr
        .iter()
        .filter_map(|h| match h.as_str() {
            "hdr10" => Some("HDR10"),
            "hdr10+" => Some("HDR10+"),
            "hlg" => Some("HLG"),
            "dv" => Some("Dolby Vision"),
            _ => None,
        })
        .collect();
    let audio: Vec<&str> = c
        .audio
        .iter()
        .filter_map(|a| match a.as_str() {
            "ac3" => Some("Dolby Digital"),
            "eac3" => Some("Dolby Digital Plus"),
            "atmos" => Some("Atmos"),
            "dts" => Some("DTS"),
            "dts-hd" => Some("DTS-HD"),
            "truehd" => Some("TrueHD"),
            _ => None,
        })
        .collect();
    let video = if hdr.is_empty() { "Screen: no HDR reported".to_string() } else { format!("Screen: {}", hdr.join(", ")) };
    let sound = if audio.is_empty() { "Sound: decoded by Encheo".to_string() } else { format!("Sound: {}", audio.join(", ")) };
    format!("{video} · {sound}")
}

/// The decoder is drawing onto the video layer (mpv mustn't draw into Slint's frame).
pub fn direct() -> bool {
    DIRECT.load(Ordering::Relaxed)
}

/// Four times a second while a video plays: choose the path for it. Returns the subtitle line
/// for Encheo to draw while on the direct path, None otherwise.
pub fn follow(enabled: bool) -> Option<String> {
    let mpv = *MPV.get()?;
    CLASS.get()?;
    let caps = CAPS.get()?;
    let forced = FORCED.get().copied().unwrap_or(false);
    let want = FILE.with(|f| {
        let mut f = f.borrow_mut();
        let path = mpv.get_property::<String>("path").unwrap_or_default();
        if f.path != path {
            *f = File { path, ..File::default() };
        }
        if f.gamma.is_none() {
            f.gamma = mpv.get_property::<String>("video-params/gamma").ok();
        }
        // The hardware decoder couldn't take this file: back to mpv's own decoding and drawing.
        if let Some(since) = f.direct_since
            && since.elapsed() > Duration::from_secs(4)
            && !f.failed
            && direct()
            && mpv.get_property::<String>("hwdec-current").is_ok_and(|h| h != "mediacodec")
        {
            eprintln!("direct video: no hardware decoding for this file, going back");
            f.failed = true;
        }
        let shows = match f.gamma.as_deref() {
            Some("pq") => caps.hdr.iter().any(|h| h == "hdr10" || h == "hdr10+"),
            Some("hlg") => caps.hdr.iter().any(|h| h == "hlg"),
            Some(_) => forced,
            None => false,
        };
        let image_subs = mpv.get_property::<String>("current-tracks/sub/codec").is_ok_and(|c| IMAGE_SUBS.contains(&c.as_str()));
        let want = (enabled || forced) && shows && !image_subs && !f.failed;
        if want && f.direct_since.is_none() {
            f.direct_since = Some(Instant::now());
        } else if !want {
            f.direct_since = None;
        }
        want
    });
    set(want);
    want.then(|| mpv.get_property::<String>("sub-text").unwrap_or_default())
}

/// Playback ended: back to the normal path.
pub fn stop() {
    if CLASS.get().is_some() {
        set(false);
    }
    FILE.with(|f| *f.borrow_mut() = File::default());
}

fn set(want: bool) {
    let Some(&mpv) = MPV.get() else { return };
    let show = {
        let mut layer = LAYER.lock().unwrap_or_else(|e| e.into_inner());
        if layer.want == want {
            return;
        }
        layer.want = want;
        if want {
            if let Some(s) = layer.surface.as_ref().map(|s| s.as_obj().as_raw() as i64) {
                to_layer(mpv, &mut layer, s);
            }
        } else {
            to_frame(mpv, &mut layer);
        }
        want
    };
    // Showing the layer creates its surface, which arrives in `surface` (Java's thread).
    with_env(|env, activity, class| {
        env.call_static_method(class, "showVideo", "(Landroid/app/Activity;Z)V", &[JValue::Object(activity), JValue::Bool(show.into())])
            .map(drop)
    });
    // The last frame Slint drew still has mpv's picture in it.
    crate::ui().window().request_redraw();
}

/// The decoder draws onto the layer.
fn to_layer(mpv: &Mpv, layer: &mut Layer, surface: i64) {
    if layer.mode == Mode::Normal {
        layer.hwdec = mpv.get_property::<String>("hwdec").unwrap_or_else(|_| "auto-safe".into());
    }
    let _ = mpv.set_property("wid", surface);
    let _ = mpv.set_property("hwdec", "mediacodec");
    if let Err(e) = mpv.set_property("vo", "mediacodec_embed") {
        eprintln!("direct video: {e}");
    }
    layer.mode = Mode::Direct;
    DIRECT.store(true, Ordering::Relaxed);
}

/// mpv draws into Slint's frame again.
fn to_frame(mpv: &Mpv, layer: &mut Layer) {
    if layer.mode == Mode::Normal {
        return;
    }
    let _ = mpv.set_property("vo", "libmpv");
    let _ = mpv.set_property("hwdec", layer.hwdec.as_str());
    let _ = mpv.set_property("wid", 0i64);
    layer.mode = Mode::Normal;
    DIRECT.store(false, Ordering::Relaxed);
}

/// `EncheoActivity.surface`, on Java's main thread: the layer's surface appeared or is going away.
extern "system" fn surface<'local>(env: JNIEnv<'local>, _class: JClass<'local>, s: JObject<'local>) {
    let Some(&mpv) = MPV.get() else { return };
    let mut layer = LAYER.lock().unwrap_or_else(|e| e.into_inner());
    if s.is_null() {
        // Java frees it as soon as this returns: mpv has to let go first.
        if layer.mode == Mode::Direct {
            let _ = mpv.set_property("vo", "null");
            let _ = mpv.set_property("wid", 0i64);
            layer.mode = Mode::Suspended;
        }
        layer.surface = None;
        return;
    }
    let Ok(global) = env.new_global_ref(&s) else { return };
    let raw = global.as_obj().as_raw() as i64;
    layer.surface = Some(global);
    if layer.want {
        to_layer(mpv, &mut layer, raw);
        drop(layer);
        let _ = slint::invoke_from_event_loop(|| crate::ui().window().request_redraw());
    }
}

/// Runs `f` with a JNI environment, the activity and `EncheoActivity`. None (after logging) on a
/// Java exception or when there's no activity.
fn with_env<R>(f: impl FnOnce(&mut JNIEnv, &JObject, &JClass) -> jni::errors::Result<R>) -> Option<R> {
    let app = crate::ANDROID_APP.get()?;
    // SAFETY: both pointers come from the running activity and stay valid while it runs.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread().ok()?;
    let r = class(&mut env, &activity).and_then(|class| f(&mut env, &activity, <&JClass>::from(class.as_obj())));
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            eprintln!("video layer: {e}");
            None
        }
    }
}

fn class(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<GlobalRef> {
    if let Some(c) = CLASS.get() {
        return Ok(c.clone());
    }
    let loader = env.call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
    let name = env.new_string("app.encheo.EncheoActivity")?;
    let class = env
        .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
        .l()?;
    let class = JClass::from(class);
    // NativeActivity loads the library itself, so Java can't find `surface` by name: register it.
    env.register_native_methods(
        &class,
        &[NativeMethod { name: "surface".into(), sig: "(Landroid/view/Surface;)V".into(), fn_ptr: surface as *mut _ }],
    )?;
    let global = env.new_global_ref(&class)?;
    Ok(CLASS.get_or_init(|| global).clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_capabilities() {
        let c = super::parse("hdr=hdr10,hlg;audio=ac3,eac3,atmos;hdmi=1");
        assert_eq!(c.hdr, ["hdr10", "hlg"]);
        assert_eq!(c.audio, ["ac3", "eac3", "atmos"]);
        let c = super::parse("hdr=;audio=;hdmi=0");
        assert!(c.hdr.is_empty() && c.audio.is_empty());
    }
}
