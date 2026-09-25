//! Raw EGL + GLES2 bindings: only what this HUD needs, nothing more.
//!
//! Linked via `#[link]` attributes — no build script, no crates. EGL 1.4
//! and GLES2 both exist on API 21+, so Android 5.1 is fully covered.

#![allow(non_camel_case_types, dead_code)]

pub type EGLDisplay = *const ();
pub type EGLSurface = *const ();
pub type EGLContext = *const ();
pub type EGLConfig = *const ();
pub type EGLint = i32;

pub const EGL_SURFACE_TYPE: EGLint = 0x3033;
pub const EGL_WINDOW_BIT: EGLint = 0x0004;
pub const EGL_BLUE_SIZE: EGLint = 0x3022;
pub const EGL_GREEN_SIZE: EGLint = 0x3023;
pub const EGL_RED_SIZE: EGLint = 0x3024;
pub const EGL_ALPHA_SIZE: EGLint = 0x3021;
pub const EGL_DEPTH_SIZE: EGLint = 0x3025;
pub const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
pub const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_WIDTH: EGLint = 0x3057;
pub const EGL_HEIGHT: EGLint = 0x3056;
pub const EGL_DISPLAY_SCALING: EGLint = 10000;
pub const EGL_NO_DISPLAY: EGLDisplay = std::ptr::null();
pub const EGL_NO_SURFACE: EGLSurface = std::ptr::null();
pub const EGL_NO_CONTEXT: EGLContext = std::ptr::null();

pub const GL_ARRAY_BUFFER: u32 = 0x8892;
pub const GL_STATIC_DRAW: u32 = 0x88E4;
pub const GL_DYNAMIC_DRAW: u32 = 0x88E8;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_TRIANGLES: u32 = 0x0004;
pub const GL_POINTS: u32 = 0x0000;
pub const GL_LINES: u32 = 0x0001;
pub const GL_VERTEX_SHADER: u32 = 0x8B31;
pub const GL_FRAGMENT_SHADER: u32 = 0x8B30;
pub const GL_COMPILE_STATUS: u32 = 0x8B81;
pub const GL_LINK_STATUS: u32 = 0x8B82;

#[link(name = "EGL")]
extern "C" {
    pub fn eglGetDisplay(display: *const ()) -> EGLDisplay;
    pub fn eglInitialize(
        display: EGLDisplay,
        major: *mut EGLint,
        minor: *mut EGLint,
    ) -> u32;
    pub fn eglChooseConfig(
        display: EGLDisplay,
        attribs: *const EGLint,
        configs: *mut EGLConfig,
        config_size: EGLint,
        num_config: *mut EGLint,
    ) -> u32;
    pub fn eglCreateContext(
        display: EGLDisplay,
        config: EGLConfig,
        share: EGLContext,
        attribs: *const EGLint,
    ) -> EGLContext;
    pub fn eglCreateWindowSurface(
        display: EGLDisplay,
        config: EGLConfig,
        window: *const (),
        attribs: *const EGLint,
    ) -> EGLSurface;
    pub fn eglMakeCurrent(
        display: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        context: EGLContext,
    ) -> u32;
    pub fn eglSwapBuffers(display: EGLDisplay, surface: EGLSurface) -> u32;
    pub fn eglQuerySurface(
        display: EGLDisplay,
        surface: EGLSurface,
        attribute: EGLint,
        value: *mut EGLint,
    ) -> u32;
    pub fn eglSwapInterval(display: EGLDisplay, interval: EGLint) -> u32;
    pub fn eglTerminate(display: EGLDisplay) -> u32;
}

#[link(name = "GLESv2")]
extern "C" {
    pub fn glViewport(x: i32, y: i32, w: i32, h: i32);
    pub fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    pub fn glClear(mask: u32);
    pub fn glCreateShader(kind: u32) -> u32;
    pub fn glShaderSource(
        shader: u32,
        count: i32,
        sources: *const *const u8,
        lengths: *const i32,
    );
    pub fn glCompileShader(shader: u32);
    pub fn glGetShaderiv(shader: u32, param: u32, value: *mut i32);
    pub fn glCreateProgram() -> u32;
    pub fn glAttachShader(program: u32, shader: u32);
    pub fn glLinkProgram(program: u32);
    pub fn glGetProgramiv(program: u32, param: u32, value: *mut i32);
    pub fn glUseProgram(program: u32);
    pub fn glGetAttribLocation(program: u32, name: *const u8) -> i32;
    pub fn glDrawArrays(mode: u32, first: i32, count: i32);
    pub fn glGenBuffers(count: i32, buffers: *mut u32);
    pub fn glBindBuffer(target: u32, buffer: u32);
    pub fn glBufferData(
        target: u32,
        size: isize,
        data: *const f32,
        usage: u32,
    );
    pub fn glEnableVertexAttribArray(index: u32);
    pub fn glVertexAttribPointer(
        index: u32,
        size: i32,
        kind: u32,
        normalized: u8,
        stride: i32,
        pointer: *const (),
    );
}

#[link(name = "log")]
extern "C" {
    fn __android_log_print(prio: i32, tag: *const u8, text: *const u8, ...);
}

/// Log to logcat under the HudRust tag (priority 3 = INFO, 4 = WARN).
pub fn alog(level: i32, msg: &str) {
    let mut tag = b"HudRust\0".to_vec();
    let mut text = msg.as_bytes().to_vec();
    text.push(0);
    unsafe {
        __android_log_print(level, tag.as_ptr(), text.as_ptr());
    }
}
