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

/// The Blade temple touchpad surfaces as directional keys; any of them
/// toggles the blank layer for now (one interaction, that's it).
#[cfg(target_os = "android")]
fn consume_input(app: &AndroidApp) {
    use android_activity::{InputStatus, input::{InputEvent, KeyAction}};
    use android_activity::input::Keycode;

    if let Ok(mut iter) = app.input_events_iter() {
        while iter.next(|event| match event {
            InputEvent::KeyEvent(key) => {
                if key.action() == KeyAction::Down {
                    if matches!(
                        key.key_code(),
                        Keycode::DpadRight
                            | Keycode::DpadLeft
                            | Keycode::DpadCenter
                            | Keycode::Back
                    ) {
                        let now = !BLANKED.load(Ordering::Relaxed);
                        BLANKED.store(now, Ordering::Relaxed);
                    }
                }
                InputStatus::Handled
            }
            _ => InputStatus::Unhandled,
        }) {}
    }
}

#[cfg(target_os = "android")]
fn init_egl(
    app: &AndroidApp,
) -> Result<(ffi::EGLDisplay, ffi::EGLSurface, renderer::Renderer), ()> {
    let window = app.native_window().ok_or(())?;
    let native = window.ptr().as_ptr() as *const ();
    unsafe {
        let display = ffi::eglGetDisplay(std::ptr::null());
        if display == ffi::EGL_NO_DISPLAY {
            return Err(());
        }
        let (mut major, mut minor) = (0, 0);
        if ffi::eglInitialize(display, &mut major, &mut minor) == 0 {
            return Err(());
        }
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
            return Err(());
        }
        // EGL_CONTEXT_CLIENT_VERSION = 2 (GLES2).
        const CONTEXT_ATTRIBS: [ffi::EGLint; 3] = [0x3098, 2, ffi::EGL_NONE];
        let context =
            ffi::eglCreateContext(display, config, ffi::EGL_NO_CONTEXT, CONTEXT_ATTRIBS.as_ptr());
        if context == ffi::EGL_NO_CONTEXT {
            return Err(());
        }
        let surface = ffi::eglCreateWindowSurface(display, config, native, std::ptr::null());
        if surface == ffi::EGL_NO_SURFACE {
            return Err(());
        }
        if ffi::eglMakeCurrent(display, surface, surface, context) == 0 {
            return Err(());
        }
        ffi::eglSwapInterval(display, 1);
        let viewport = (window.width() as i32, window.height() as i32);
        Ok((display, surface, renderer::Renderer::new(viewport)))
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
    let shared = Arc::new(poll::Shared::default());
    poll::spawn(shared.clone()).expect("spawn poll thread");

    let mut renderer: Option<renderer::Renderer> = None;
    let mut display = ffi::EGL_NO_DISPLAY;
    let mut surface = ffi::EGL_NO_SURFACE;
    let mut quit = false;

    while !quit {
        app.poll_events(Some(std::time::Duration::from_millis(FRAME_MS)), |event| {
            use android_activity::PollEvent;
            match event {
                PollEvent::Main(main) => {
                    use android_activity::MainEvent;
                    match main {
                        MainEvent::InitWindow { .. } | MainEvent::WindowResized { .. } => {
                            renderer = None;
                            if let Ok((disp, surf, ren)) = init_egl(&app) {
                                display = disp;
                                surface = surf;
                                renderer = Some(ren);
                            }
                        }
                        MainEvent::TerminateWindow { .. } => {
                            renderer = None;
                            destroy_egl(display);
                            display = ffi::EGL_NO_DISPLAY;
                            surface = ffi::EGL_NO_SURFACE;
                        }
                        MainEvent::InputAvailable => consume_input(&app),
                        MainEvent::Destroy => quit = true,
                        _ => {}
                    }
                }
                _ => {}
            }
        });

        let blanked = BLANKED.load(Ordering::Relaxed);
        if let Some(r) = renderer.as_ref().filter(|_| !blanked) {
            let model = shared.model.lock().unwrap().clone();
            let prims = model::build_frame(&model, r.viewport.0, r.viewport.1);
            let batch = renderer::Batch::build(&prims, r.viewport.0, r.viewport.1);
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
