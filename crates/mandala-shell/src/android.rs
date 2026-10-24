//! Android system integration (Phase 5): the two OS-level capabilities a
//! launcher cannot get from Rust alone.
//!
//! * **Wallpaper** – `WallpaperManager.setStream` with `SET_WALLPAPER`.
//! * **Overlay** – `Settings.canDrawOverlays` plus the
//!   `ACTION_MANAGE_OVERLAY_PERMISSION` system dialog for
//!   `SYSTEM_ALERT_WINDOW`.
//!
//! Both need the Java environment that `android-activity` publishes through
//! `ndk-context`; the host passes its `Activity` as the `Context`.
//!
//! Errors never cross the FFI boundary as panics: every failure becomes a
//! `String` the command bar can show.

use jni::objects::{JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::path::Path;

const OVERLAY_ACTION: &str = "android.settings.action.MANAGE_OVERLAY_PERMISSION";
const CAPTURE_ACTION: &str = "android.media.action.IMAGE_CAPTURE";
/// `MediaStore.EXTRA_OUTPUT` — the camera app writes the full-size image here.
const EXTRA_OUTPUT: &str = "output";
/// FLAG_GRANT_READ_URI_PERMISSION | FLAG_GRANT_WRITE_URI_PERMISSION.
const FLAG_GRANT_READ_WRITE: i32 = 0x1 | 0x2;

// The closure binds `env` and `activity` to the *same* local reference frame;
// without the higher-ranked bound Rust gives them independent anonymous
// lifetimes and `capture_uri` (which ties them together) cannot be called.
fn with_activity<T>(
    f: impl for<'local> FnOnce(&mut JNIEnv<'local>, &JObject<'local>) -> Result<T, String>,
) -> Result<T, String> {
    let ctx = ndk_context::android_context();
    // SAFETY: android-activity installs this VM/context pair before `main`.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| format!("jni: {e}"))?;
    let mut env = vm.attach_current_thread().map_err(|e| format!("jni: {e}"))?;
    // SAFETY: the context is the Activity reference published by ndk-context.
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let out = f(&mut env, &activity)?;
    check_exception(&mut env)?;
    Ok(out)
}

fn check_exception(env: &mut JNIEnv<'_>) -> Result<(), String> {
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        let _ = env.exception_clear();
        return Err("Java threw an exception (details in logcat)".into());
    }
    Ok(())
}

/// Replace the system wallpaper with a local image file.
pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    let path = path.to_str().ok_or("wallpaper path is not valid UTF-8")?.to_string();
    if path.is_empty() {
        return Err("wallpaper path is empty".into());
    }
    with_activity(move |env, activity| {
        let jpath = env.new_string(&path).map_err(|e| format!("jni: {e}"))?;
        let file = env
            .new_object("java/io/File", "(Ljava/lang/String;)V", &[JValue::Object(&jpath)])
            .map_err(|e| format!("jni: {e}"))?;
        let stream = env
            .new_object("java/io/FileInputStream", "(Ljava/io/File;)V", &[JValue::Object(&file)])
            .map_err(|e| format!("wallpaper: {e}"))?;
        let manager_class = env.find_class("android/app/WallpaperManager").map_err(|e| format!("jni: {e}"))?;
        let manager = env
            .call_static_method(
                manager_class,
                "getInstance",
                "(Landroid/content/Context;)Landroid/app/WallpaperManager;",
                &[JValue::Object(activity)],
            )
            .map_err(|e| format!("wallpaper: {e}"))?
            .l()
            .map_err(|e| format!("wallpaper: {e}"))?;
        env.call_method(manager, "setStream", "(Ljava/io/InputStream;)V", &[JValue::Object(&stream)])
            .map_err(|e| format!("wallpaper: {e}"))?;
        Ok(())
    })
}

/// Fire `ACTION_IMAGE_CAPTURE` so the user's camera app (Open Camera or
/// whatever is default) writes a full-size photo straight to `target`.
///
/// The camera app runs in its own process, so it needs a `content://` URI it
/// is allowed to write: we ask AndroidX `FileProvider` for one when the app
/// declares the provider, and fall back to `Uri.fromFile` otherwise (which
/// Android allows only below API 24 — the error surfaces verbatim if the
/// provider is missing on a modern device, instead of a silent no-op).
pub fn capture(target: &Path) -> Result<(), String> {
    let path = target.to_str().ok_or("capture path is not valid UTF-8")?.to_string();
    with_activity(move |env, activity| {
        let jpath = env.new_string(&path).map_err(|e| format!("jni: {e}"))?;
        let file = env
            .new_object("java/io/File", "(Ljava/lang/String;)V", &[JValue::Object(&jpath)])
            .map_err(|e| format!("camera: {e}"))?;
        let uri = capture_uri(env, activity, &file)?;
        let action = env.new_string(CAPTURE_ACTION).map_err(|e| format!("jni: {e}"))?;
        let intent = env
            .new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])
            .map_err(|e| format!("camera: {e}"))?;
        let key = env.new_string(EXTRA_OUTPUT).map_err(|e| format!("jni: {e}"))?;
        env.call_method(
            &intent,
            "putExtra",
            "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
            &[JValue::Object(&key), JValue::Object(&uri)],
        )
        .map_err(|e| format!("camera: {e}"))?;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(FLAG_GRANT_READ_WRITE)])
            .map_err(|e| format!("camera: {e}"))?;
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])
            .map_err(|e| format!("camera: {e}"))?;
        Ok(())
    })
}

/// `content://<package>.fileprovider/...` when available, else `file://`.
fn capture_uri<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
    file: &JObject<'local>,
) -> Result<JObject<'local>, String> {
    if let Ok(class) = env.find_class("androidx/core/content/FileProvider") {
        let package = env
            .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])
            .map_err(|e| format!("camera: {e}"))?
            .l()
            .map_err(|e| format!("camera: {e}"))?;
        let package = JString::from(package);
        let package: String = env.get_string(&package).map_err(|e| format!("camera: {e}"))?.into();
        let authority = env.new_string(format!("{package}.fileprovider")).map_err(|e| format!("jni: {e}"))?;
        let uri = env.call_static_method(
            class,
            "getUriForFile",
            "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
            &[JValue::Object(activity), JValue::Object(&authority), JValue::Object(file)],
        );
        match uri.and_then(|v| v.l()) {
            Ok(uri) => return Ok(uri),
            Err(_) => {
                // Provider declared but not resolving: clear and fall back.
                let _ = env.exception_clear();
            }
        }
    } else {
        // No AndroidX on the classpath — clear the pending class-not-found.
        let _ = env.exception_clear();
    }
    let uri_class = env.find_class("android/net/Uri").map_err(|e| format!("jni: {e}"))?;
    env.call_static_method(uri_class, "fromFile", "(Ljava/io/File;)Landroid/net/Uri;", &[JValue::Object(file)])
        .map_err(|e| format!("camera: {e} (no FileProvider declared in the manifest; add android/res/xml/file_paths.xml)"))?
        .l()
        .map_err(|e| format!("camera: {e}"))
}

/// True when the app may draw over other apps.
pub fn overlay_granted() -> Result<bool, String> {
    with_activity(|env, activity| {
        let settings = env.find_class("android/provider/Settings").map_err(|e| format!("jni: {e}"))?;
        let granted = env
            .call_static_method(
                settings,
                "canDrawOverlays",
                "(Landroid/content/Context;)Z",
                &[JValue::Object(activity)],
            )
            .map_err(|e| format!("overlay: {e}"))?
            .z()
            .map_err(|e| format!("overlay: {e}"))?;
        Ok(granted)
    })
}

/// Open the system screen that grants `SYSTEM_ALERT_WINDOW`.
/// Returns `true` when the permission was already granted.
pub fn request_overlay_permission() -> Result<bool, String> {
    if overlay_granted()? {
        return Ok(true);
    }
    with_activity(|env, activity| {
        let package = env
            .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])
            .map_err(|e| format!("overlay: {e}"))?
            .l()
            .map_err(|e| format!("overlay: {e}"))?;
        let package = JString::from(package);
        let package: String = env.get_string(&package).map_err(|e| format!("overlay: {e}"))?.into();
        let uri_text = env.new_string(format!("package:{package}")).map_err(|e| format!("jni: {e}"))?;
        let uri_class = env.find_class("android/net/Uri").map_err(|e| format!("jni: {e}"))?;
        let uri = env
            .call_static_method(uri_class, "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&uri_text)])
            .map_err(|e| format!("overlay: {e}"))?
            .l()
            .map_err(|e| format!("overlay: {e}"))?;
        let action = env.new_string(OVERLAY_ACTION).map_err(|e| format!("jni: {e}"))?;
        let intent = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )
            .map_err(|e| format!("overlay: {e}"))?;
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])
            .map_err(|e| format!("overlay: {e}"))?;
        Ok(false)
    })
}
