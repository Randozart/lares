//! Head-mounted capture: temple tap → camera → JPEG bytes.
//!
//! Bridges to the Java CameraShim (the API 22 camera is framework-only)
//! over JNI. One capture per call; the camera powers on and off inside
//! the shim, so idle battery cost is zero.

use jni::objects::JByteArray;
use jni::JNIEnv;

use crate::ffi;

/// Capture one JPEG through the Java shim. None on any failure.
///
/// The shim class is resolved through the activity's classloader: a
/// native-attached thread's FindClass only sees the system loader and
/// cannot find application classes.
pub fn capture_jpeg(
    vm_ptr: *mut std::ffi::c_void,
    activity_ptr: *mut std::ffi::c_void,
) -> Option<Vec<u8>> {
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;

    ffi::alog(3, "camera: capturing");
    let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr.cast()) };
    let loader = env
        .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .ok()?
        .l()
        .ok()?;
    let name = env.new_string("dev.randozart.lares.hud.CameraShim").ok()?;
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[(&name).into()],
        )
        .ok()?
        .l()
        .ok()?;
    let class = jni::objects::JClass::from(class);
    let value = match env.call_static_method(class, "capture", "()[B", &[]) {
        Ok(value) => value,
        Err(_) => {
            // Describe and clear the pending Java exception.
            let throwable = env.exception_occurred().ok();
            env.exception_clear();
            if let Some(t) = throwable {
                let desc = env
                    .call_method(&t, "toString", "()Ljava/lang/String;", &[])
                    .ok()
                    .and_then(|s| s.l().ok())
                    .and_then(|s| {
                        let js = jni::objects::JString::from(s);
                        let out = env.get_string(&js).ok()?.to_string_lossy().into_owned();
                        Some(out)
                    });
                ffi::alog(4, &format!("camera: java exception: {:?}", desc));
            } else {
                ffi::alog(4, "camera: java exception (undescribed)");
            }
            return None;
        }
    };

    let object = value.l().ok()?;
    if object.is_null() {
        ffi::alog(4, "camera: null frame");
        return None;
    }

    let array = unsafe { JByteArray::from_raw(object.as_raw()) };
    let length = env.get_array_length(&array).ok()? as usize;
    if length == 0 {
        ffi::alog(4, "camera: empty frame");
        return None;
    }
    let mut buffer = vec![0i8; length];
    if env.get_byte_array_region(&array, 0, &mut buffer).is_err() {
        ffi::alog(4, "camera: copy failed");
        return None;
    }
    ffi::alog(3, &format!("camera: {} bytes", length));

    Some(buffer.into_iter().map(|b| b as u8).collect())
}
