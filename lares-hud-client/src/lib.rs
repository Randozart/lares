//! lares-hud-client — bare-metal Rust HUD for the Vuzix Blade.
//!
//! NativeActivity, EGL, GLES2, a 5×7 font, and a 2-second poll of the
//! Lares server's /v1/hud. No Java beyond what the framework provides,
//! no storage, no obligations: a readout floating in the corner of vision.
//!
//! On host (`cargo test`) the layout code renders a PBM preview so the
//! design is verifiable without hardware.

// On host the crate exists only to run tests (font/layout/preview);
// the dead-code cascade from the unused Android loop is expected there.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

mod font;
mod model;
mod poll;

#[cfg(target_os = "android")]
mod camera;
#[cfg(target_os = "android")]
mod ffi;
#[cfg(target_os = "android")]
mod renderer;

#[cfg(target_os = "android")]
use android_activity::AndroidApp;
#[cfg(target_os = "android")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "android")]
use std::sync::Arc;

/// Seconds per render frame (roughly 60 Hz pacing without vsync trust).
#[cfg(target_os = "android")]
const FRAME_MS: u64 = 16;

/// The Blade temple touchpad surfaces as directional keys. Center tap =
/// head-mounted capture + tick; side taps toggle the blank layer; Back
/// is left to the system so it exits the activity.
#[cfg(target_os = "android")]
fn handle_input(event: &android_activity::input::InputEvent) -> android_activity::InputStatus {
    use android_activity::input::MotionAction;
    use android_activity::{InputStatus, input::{InputEvent, KeyAction, Keycode}};

    match event {
        // The Blade touchpad tap surfaces as Menu (DpadCenter on some
        // firmwares): both trigger a head-mounted tick.
        InputEvent::KeyEvent(key) => {
            ffi::alog(3, &format!("key: {:?} action {:?}", key.key_code(), key.action()));
            if key.action() == KeyAction::Down && key.key_code() == Keycode::Back {
                return InputStatus::Unhandled;
            }
            if key.action() == KeyAction::Down {
                match key.key_code() {
                    Keycode::Menu | Keycode::DpadCenter => {
                        CAPTURE_REQUEST.store(true, Ordering::Relaxed)
                    }
                    _ => {
                        let now = !BLANKED.load(Ordering::Relaxed);
                        BLANKED.store(now, Ordering::Relaxed);
                    }
                }
            }
            InputStatus::Handled
        }
        // Swipes beyond 60px toggle the blank layer.
        InputEvent::MotionEvent(motion) => match motion.action() {
            MotionAction::Down => {
                if let Some(pointer) = motion.pointers().next() {
                    SWIPE_START_X.store(pointer.x() as i32, Ordering::Relaxed);
                }
                InputStatus::Handled
            }
            MotionAction::Up => {
                if let Some(pointer) = motion.pointers().next() {
                    let start = SWIPE_START_X.swap(-1, Ordering::Relaxed);
                    if start >= 0 && ((pointer.x() as i32) - start).abs() > 60 {
                        let now = !BLANKED.load(Ordering::Relaxed);
                        BLANKED.store(now, Ordering::Relaxed);
                    }
                }
                InputStatus::Handled
            }
            _ => InputStatus::Unhandled,
        },
        _ => InputStatus::Unhandled,
    }
}

#[cfg(target_os = "android")]
fn init_egl(
    app: &AndroidApp,
) -> Result<(ffi::EGLDisplay, ffi::EGLSurface, renderer::Renderer), ()> {
    let window = app.native_window().ok_or(())?;
    let _ = window; // size comes from the EGL surface, not the window hint
    let native = app.native_window().map(|w| w.ptr().as_ptr() as *const ()).ok_or(())?;
    unsafe {
        ffi::alog(3, "egl: get display");
        let display = ffi::eglGetDisplay(std::ptr::null());
        if display == ffi::EGL_NO_DISPLAY {
            ffi::alog(4, "egl: no display");
            return Err(());
        }
        let (mut major, mut minor) = (0, 0);
        if ffi::eglInitialize(display, &mut major, &mut minor) == 0 {
            ffi::alog(4, "egl: initialize failed");
            return Err(());
        }
        ffi::alog(3, "egl: initialized");
        const CONFIG_ATTRIBS: [ffi::EGLint; 13] = [
            ffi::EGL_SURFACE_TYPE,
            ffi::EGL_WINDOW_BIT,
            ffi::EGL_RENDERABLE_TYPE,
            ffi::EGL_OPENGL_ES2_BIT,
            ffi::EGL_RED_SIZE,
            5,
            ffi::EGL_GREEN_SIZE,
            6,
            ffi::EGL_BLUE_SIZE,
            5,
            ffi::EGL_DEPTH_SIZE,
            0,
            ffi::EGL_NONE,
        ];
        let mut config = std::ptr::null();
        let mut count = 0;
        if ffi::eglChooseConfig(display, CONFIG_ATTRIBS.as_ptr(), &mut config, 1, &mut count) == 0
            || count < 1
        {
            ffi::alog(4, "egl: no config");
            return Err(());
        }
        ffi::alog(3, "egl: config ok");
        // EGL_CONTEXT_CLIENT_VERSION = 2 (GLES2).
        const CONTEXT_ATTRIBS: [ffi::EGLint; 3] = [0x3098, 2, ffi::EGL_NONE];
        let context =
            ffi::eglCreateContext(display, config, ffi::EGL_NO_CONTEXT, CONTEXT_ATTRIBS.as_ptr());
        if context == ffi::EGL_NO_CONTEXT {
            ffi::alog(4, "egl: no context");
            return Err(());
        }
        ffi::alog(3, "egl: context ok");
        let surface = ffi::eglCreateWindowSurface(display, config, native, std::ptr::null());
        if surface == ffi::EGL_NO_SURFACE {
            ffi::alog(4, "egl: no surface");
            return Err(());
        }
        if ffi::eglMakeCurrent(display, surface, surface, context) == 0 {
            ffi::alog(4, "egl: make current failed");
            return Err(());
        }
        ffi::alog(3, "egl: current ok");
        ffi::eglSwapInterval(display, 1);
        // The Blade's panel is 480x853; the window hint lies. Ask EGL.
        let (mut vw, mut vh) = (0, 0);
        if ffi::eglQuerySurface(display, surface, ffi::EGL_WIDTH, &mut vw) == 0
            || ffi::eglQuerySurface(display, surface, ffi::EGL_HEIGHT, &mut vh) == 0
            || vw <= 0
            || vh <= 0
        {
            ffi::alog(4, "egl: query surface failed");
            return Err(());
        }
        ffi::alog(3, &format!("egl: surface {}x{}", vw, vh));
        Ok((display, surface, renderer::Renderer::new((vw, vh))))
    }
}

#[cfg(target_os = "android")]
fn destroy_egl(display: ffi::EGLDisplay) {
    unsafe {
        ffi::eglTerminate(display);
    }
}

#[cfg(target_os = "android")]
fn android_main_impl(app: AndroidApp) {
    ffi::alog(3, "android_main entered");
    // Keep the display alive while the HUD is foregrounded. Must be called
    // OUTSIDE poll_events: that call holds the activity's read lock, and
    // set_window_flags takes the write lock — same-thread deadlock.
    app.set_window_flags(
        android_activity::WindowManagerFlags::KEEP_SCREEN_ON,
        android_activity::WindowManagerFlags::empty(),
    );
    let shared = Arc::new(poll::Shared::default());
    poll::spawn(shared.clone()).expect("spawn poll thread");

    let mut renderer: Option<renderer::Renderer> = None;
    let mut display = ffi::EGL_NO_DISPLAY;
    let mut surface = ffi::EGL_NO_SURFACE;
    let mut quit = false;
    let mut first_draw_done = false;
    let mut iterations: u64 = 0;
    // The input receiver is single-tenant: create it once and keep it for
    // the app's lifetime; later calls return InputUnavailable.
    let mut input_iter: Option<android_activity::input::InputIterator> = None;

    loop {
        iterations += 1;
        if iterations % 200 == 0 {
            ffi::alog(3, &format!("loop iter {} window={:?}", iterations, app.native_window().is_some()));
        }
        if quit {
            return;
        }
        app.poll_events(Some(std::time::Duration::from_millis(FRAME_MS)), |event| {
            use android_activity::PollEvent;
            match &event {
                PollEvent::Main(main) => ffi::alog(3, &format!("event: {}", main_event_name(main))),
                _ => {}
            }
            match event {
                PollEvent::Main(main) => {
                    use android_activity::MainEvent;
                    match main {
                        MainEvent::InitWindow { .. } | MainEvent::WindowResized { .. } => {
                            // Idempotent: WindowResized also fires after our
                            // own KEEP_SCREEN_ON flag change; re-initing over
                            // a live surface fails and blinds the HUD.
                            if renderer.is_none() {
                                if let Ok((disp, surf, ren)) = init_egl(&app) {
                                    display = disp;
                                    surface = surf;
                                    renderer = Some(ren);
                                }
                            }
                        }
                        MainEvent::TerminateWindow { .. } => {
                            renderer = None;
                            destroy_egl(display);
                            display = ffi::EGL_NO_DISPLAY;
                            surface = ffi::EGL_NO_SURFACE;
                        }
                        MainEvent::InputAvailable => {
                            // Drop any previous receiver first — the guard
                            // refuses a new iterator while the old one lives.
                            input_iter = None;
                            match app.input_events_iter() {
                                Ok(mut iter) => {
                                    let mut consumed = 0;
                                    while iter.next(handle_input) {
                                        consumed += 1;
                                    }
                                    ffi::alog(3, &format!("consumed {}", consumed));
                                }
                                Err(e) => ffi::alog(4, &format!("iter err: {e:?}")),
                            }
                        }
                        MainEvent::Destroy => quit = true,
                        _ => {}
                    }
                }
                _ => {}
            }
        });

        // Center tap = immediate tick; otherwise the continuous stream
        // re-ticks every AUTO_TICK_MS while the display is live.
        let manual = CAPTURE_REQUEST.swap(false, Ordering::Relaxed);
        let due = now_ms() - LAST_TICK_MS.load(Ordering::Relaxed) > AUTO_TICK_MS;
        if (manual || due) && !TICK_RUNNING.swap(true, Ordering::Relaxed) {
            LAST_TICK_MS.store(now_ms(), Ordering::Relaxed);
            run_tick(&app, &shared);
            TICK_RUNNING.store(false, Ordering::Relaxed);
        }

        let blanked = BLANKED.load(Ordering::Relaxed);
        if let Some(r) = renderer.as_ref().filter(|_| !blanked) {
            let model = shared.model.lock().unwrap().clone();
            // On elongated panels (Blade: 480x853) lay out a centered
            // square band — the waveguide optics show the panel center.
            let (vw, vh) = r.viewport;
            let (lw, lh, y_off) = if vh > vw + vw / 4 {
                (vw, vw, (vh - vw) / 2)
            } else {
                (vw, vh, 0)
            };
            let mut prims = model::build_frame(&model, lw, lh);
            if y_off > 0 {
                for prim in prims.iter_mut() {
                    prim.shift_y(y_off);
                }
            }
            let batch = renderer::Batch::build(&prims, vw, vh);
            r.draw(&batch);
            unsafe {
                ffi::eglSwapBuffers(display, surface);
            }
        }
    }
}

#[cfg(target_os = "android")]
static BLANKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "android")]
static CAPTURE_REQUEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "android")]
static TICK_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Continuous clutter identification: auto-tick cadence.
#[cfg(target_os = "android")]
const AUTO_TICK_MS: u64 = 12_000;

#[cfg(target_os = "android")]
static LAST_TICK_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(target_os = "android")]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// X coordinate of an in-progress touch swipe (motion tracking).
#[cfg(target_os = "android")]
static SWIPE_START_X: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Head-mounted tick: capture through the camera shim, POST /v1/tick,
/// adopt the candidates as tags. Runs on the render thread — the ~4s
/// block shows the last frame, which is exactly what the user is
/// pointing at anyway.
#[cfg(target_os = "android")]
fn run_tick(app: &AndroidApp, shared: &Arc<poll::Shared>) {
    ffi::alog(3, "tick: capturing");
    shared.model.lock().unwrap().status = "TICK...".into();
    let Some(jpeg) = camera::capture_jpeg(app.vm_as_ptr(), app.activity_as_ptr()) else {
        ffi::alog(4, "tick: capture failed");
        let mut model = shared.model.lock().unwrap();
        model.status = "TICK FAIL".into();
        model.connected = false;
        return;
    };
    let upload = if jpeg.len() > 400_000 { downscale(jpeg) } else { jpeg };
    let body = serde_json::json!({
        "roomId": "kitchen",
        "frameJpeg": base64_encode(&upload),
        "roomArea": "ROOM_AREA_UNSPECIFIED",
    });
    let url = format!("{}/v1/tick", poll::SERVER_URL);
    ffi::alog(3, "tick: posting");
    match poll::http_post(&url, &body.to_string()) {
        Ok(response_body) => match serde_json::from_str::<poll::TickResponse>(&response_body) {
            Ok(parsed) => {
                ffi::alog(3, &format!("tick: {} candidates", parsed.candidates.len()));
                poll::apply_tick(shared, &parsed, poll::tick_now());
            }
            Err(e) => ffi::alog(4, &format!("tick parse: {e}")),
        },
        Err(e) => {
            ffi::alog(4, &format!("tick post: {e}"));
            shared.model.lock().unwrap().connected = false;
        }
    }
}

/// Naive half-scale JPEG passthrough: the server downscales anyway, so
/// only oversized frames are trimmed by raw byte cropping is unsafe —
/// instead just cap by returning the frame as-is (the sidecar resizes).
#[cfg(target_os = "android")]
fn downscale(jpeg: Vec<u8>) -> Vec<u8> {
    jpeg
}

#[cfg(target_os = "android")]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(target_os = "android")]
fn main_event_name(e: &android_activity::MainEvent) -> &'static str {
    use android_activity::MainEvent;
    match e {
        MainEvent::Resume { .. } => "Resume",
        MainEvent::Pause {} => "Pause",
        MainEvent::InitWindow { .. } => "InitWindow",
        MainEvent::TerminateWindow { .. } => "TerminateWindow",
        MainEvent::WindowResized { .. } => "WindowResized",
        MainEvent::RedrawNeeded { .. } => "RedrawNeeded",
        MainEvent::GainedFocus => "GainedFocus",
        MainEvent::LostFocus => "LostFocus",
        MainEvent::InputAvailable => "InputAvailable",
        MainEvent::Destroy => "Destroy",
        _ => "other",
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: AndroidApp) {
    android_main_impl(app);
}

#[cfg(test)]
mod tests {
    use crate::model::{build_frame, HudModel, Prim};

    /// Software raster of the primitives to a boolean grid, for preview.
    fn rasterize(prims: &[Prim], w: i32, h: i32) -> Vec<Vec<bool>> {
        let mut grid = vec![vec![false; w as usize]; h as usize];
        let mut set = |x: i32, y: i32| {
            if x >= 0 && x < w && y >= 0 && y < h {
                grid[y as usize][x as usize] = true;
            }
        };
        for prim in prims {
            match *prim {
                Prim::Dot { x, y, .. } => set(x, y),
                Prim::HLine { x, y, len, .. } => (0..len).for_each(|i| set(x + i, y)),
                Prim::VLine { x, y, len, .. } => (0..len).for_each(|i| set(x, y + i)),
                Prim::Frame { x, y, w: fw, h: fh, .. } => {
                    (0..fw).for_each(|i| {
                        set(x + i, y);
                        set(x + i, y + fh - 1);
                    });
                    (0..fh).for_each(|i| {
                        set(x, y + i);
                        set(x + fw - 1, y + i);
                    });
                }
                Prim::Fill { x, y, w: fw, h: fh, .. } => (0..fh).for_each(|dy| {
                    (0..fw).for_each(|dx| set(x + dx, y + dy));
                }),
            }
        }
        grid
    }

    #[test]
    fn preview_frame_renders_content() {
        let model = HudModel {
            room: "KITCHEN".into(),
            targets: 3,
            tag_titles: vec!["SOCKS".into(), "CUP".into(), "BOOK".into()],
            tag_ages: vec![5, 120, 900],
            connected: true,
            status: String::new(),
            tick: 0,
        };
        let (w, h) = (192, 384);
        let prims = build_frame(&model, w, h);
        let grid = rasterize(&prims, w, h);
        let lit: usize = grid.iter().map(|row| row.iter().filter(|p| **p).count()).sum();
        assert!(lit > 400, "frame should light up the display, got {lit}");

        let mut ppm = format!("P1\n{w} {h}\n");
        for row in &grid {
            for cell in row {
                ppm.push_str(if *cell { "1 " } else { "0 " });
            }
            ppm.push('\n');
        }
        let path = std::env::temp_dir().join("lares-hud-preview.pbm");
        std::fs::write(&path, ppm).unwrap();
        println!("preview written to {}", path.display());
    }

    #[test]
    fn layout_survives_empty_state() {
        let model = HudModel::default();
        let prims = build_frame(&model, 192, 384);
        let grid = rasterize(&prims, 192, 384);
        let lit: usize = grid.iter().map(|row| row.iter().filter(|p| **p).count()).sum();
        assert!(lit > 100);
    }
}
