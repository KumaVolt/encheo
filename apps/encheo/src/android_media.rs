//! Android: music on the lock screen, in the notification shade and on headphone buttons, and
//! kept playing with the screen off. The Java side is `MusicService` (android/app/src/main/java);
//! this sends it the song and receives its buttons.

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM, NativeMethod};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// `MusicService`, loaded through the app's class loader (a native thread only sees the system's).
static CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// What was last sent: the song, whether it played, and where it was when.
struct Sent {
    key: String,
    playing: bool,
    position: f64,
    at: Instant,
}
static SENT: Mutex<Option<Sent>> = Mutex::new(None);

/// The song that's playing. Cheap to call often: only sends when the song or play/pause changes,
/// or the position jumps (a seek).
pub fn sync(key: &str, title: &str, artist: &str, album: &str, cover: &str, duration: f64, position: f64, playing: bool) {
    // (mpv reports a slightly negative time at the start of a song.)
    let position = position.max(0.0);
    {
        let mut sent = SENT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = sent.as_ref() {
            let expected = if s.playing { s.position + s.at.elapsed().as_secs_f64() } else { s.position };
            if s.key == key && s.playing == playing && (position - expected).abs() < 2.0 {
                return;
            }
        }
        *sent = Some(Sent { key: key.to_string(), playing, position, at: Instant::now() });
    }
    let r = with_env(|env, activity, class| {
        let strings = [title, artist, album, cover].map(|s| env.new_string(s));
        let [t, ar, al, c] = strings;
        let (t, ar, al, c) = (t?, ar?, al?, c?);
        env.call_static_method(
            class,
            "update",
            "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJZ)V",
            &[
                JValue::Object(activity),
                JValue::Object(&t),
                JValue::Object(&ar),
                JValue::Object(&al),
                JValue::Object(&c),
                JValue::Long((duration * 1000.0) as i64),
                JValue::Long((position * 1000.0) as i64),
                JValue::Bool(playing.into()),
            ],
        )?;
        Ok(())
    });
    if r.is_none() {
        // Try again next time.
        *SENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Music closed: take the controls away.
pub fn stop() {
    if SENT.lock().unwrap_or_else(|e| e.into_inner()).take().is_none() {
        return;
    }
    with_env(|env, _, class| env.call_static_method(class, "stop", "()V", &[]).map(drop));
}

/// Runs `f` with a JNI environment, the activity and `MusicService`. None (after logging) on a
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
            eprintln!("music service: {e}");
            None
        }
    }
}

fn class(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<GlobalRef> {
    if let Some(c) = CLASS.get() {
        return Ok(c.clone());
    }
    let loader = env.call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
    let name = env.new_string("app.encheo.MusicService")?;
    let class = env
        .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
        .l()?;
    let class = JClass::from(class);
    // NativeActivity loads the library itself, so Java can't find `command` by name: register it.
    env.register_native_methods(
        &class,
        &[NativeMethod { name: "command".into(), sig: "(Ljava/lang/String;)V".into(), fn_ptr: command as *mut _ }],
    )?;
    let global = env.new_global_ref(&class)?;
    Ok(CLASS.get_or_init(|| global).clone())
}

/// `MusicService.command`: a button on the lock screen, the notification or headphones.
extern "system" fn command<'local>(mut env: JNIEnv<'local>, _class: JClass<'local>, cmd: JString<'local>) {
    let Ok(cmd) = env.get_string(&cmd).map(String::from) else { return };
    let _ = slint::invoke_from_event_loop(move || crate::media_command(&cmd));
}
