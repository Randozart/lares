//! GLES2 renderer: turns the layout's primitives into vertex batches.
//!
//! One passthrough program, one dynamic vertex buffer, three draw calls
//! (points for font pixels, lines for structure, quads as triangles for
//! fills). Every primitive expands to triangles/points with a per-vertex
//! color; the waveguide only carries luminance but the math keeps the
//! amber intent as ratios.

use crate::ffi as gl;
use crate::model::{Level, Prim};

/// Vertices per primitive kind laid out as (x, y, r, g, b) triplets.
pub struct Batch {
    pub data: Vec<f32>,
    points: usize,
    lines: usize,
    tris: usize,
}

/// Luminance ratios per level — the mono waveguide collapses these anyway.
fn level_rgb(level: Level) -> (f32, f32, f32) {
    match level {
        Level::Full => (0.0, 1.0, 0.45),
        Level::Mid => (0.0, 0.55, 0.25),
        Level::Dim => (0.0, 0.25, 0.12),
    }
}

impl Batch {
    /// Expand a frame's primitives into draw batches for a given size.
    pub fn build(prims: &[Prim], w: i32, h: i32) -> Self {
        let mut batch = Batch {
            data: Vec::with_capacity(prims.len() * 64),
            points: 0,
            lines: 0,
            tris: 0,
        };
        for prim in prims {
            batch.push_prim(prim, w, h);
        }
        batch
    }

    fn dot(&mut self, x: f32, y: f32, rgb: (f32, f32, f32)) {
        self.data.extend_from_slice(&[x, y, rgb.0, rgb.1, rgb.2]);
        self.points += 1;
    }

    fn tri(&mut self, pts: [(f32, f32); 3], rgb: (f32, f32, f32)) {
        for (x, y) in pts {
            self.data.extend_from_slice(&[x, y, rgb.0, rgb.1, rgb.2]);
        }
        self.tris += 3;
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), rgb: (f32, f32, f32)) {
        for (x, y) in [a, b] {
            self.data.extend_from_slice(&[x, y, rgb.0, rgb.1, rgb.2]);
        }
        self.lines += 2;
    }

    fn push_prim(&mut self, prim: &Prim, w: i32, h: i32) {
        let (wf, hf) = (w as f32, h as f32);
        let px = |x: i32, y: i32| ((x as f32 + 0.5) / wf * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / hf * 2.0);
        match *prim {
            Prim::Dot { x, y, level } => {
                let (x, y) = px(x, y);
                self.dot(x, y, level_rgb(level));
            }
            Prim::HLine { x, y, len, level } => {
                let rgb = level_rgb(level);
                let (ax, ay) = px(x, y);
                let (bx, by) = px(x + len - 1, y);
                self.line((ax, ay), (bx, by), rgb);
            }
            Prim::VLine { x, y, len, level } => {
                let rgb = level_rgb(level);
                let (ax, ay) = px(x, y);
                let (bx, by) = px(x, y + len - 1);
                self.line((ax, ay), (bx, by), rgb);
            }
            Prim::Frame { x, y, w: fw, h: fh, level } => {
                let rgb = level_rgb(level);
                let (x0, y0) = px(x, y);
                let (x1, y1) = px(x + fw - 1, y + fh - 1);
                self.line((x0, y0), (x1, y0), rgb);
                self.line((x1, y0), (x1, y1), rgb);
                self.line((x1, y1), (x0, y1), rgb);
                self.line((x0, y1), (x0, y0), rgb);
            }
            Prim::Fill { x, y, w: fw, h: fh, level } => {
                let rgb = level_rgb(level);
                let (x0, y0) = px(x, y);
                let (x1, y1) = px(x + fw - 1, y + fh - 1);
                self.tri([(x0, y0), (x1, y0), (x1, y1)], rgb);
                self.tri([(x0, y0), (x1, y1), (x0, y1)], rgb);
            }
        }
    }
}

/// Compiled shader program plus the dynamic buffer.
pub struct Renderer {
    program: u32,
    attrib_pos: i32,
    attrib_col: i32,
    buffer: u32,
    pub viewport: (i32, i32),
}

const VERTEX_SRC: &[u8] = b"
attribute vec2 a_pos;
attribute vec3 a_col;
varying vec3 v_col;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    gl_PointSize = 2.0;
    v_col = a_col;
}
\0";

const FRAGMENT_SRC: &[u8] = b"
precision mediump float;
varying vec3 v_col;
void main() {
    gl_FragColor = vec4(v_col, 1.0);
}
\0";


impl Renderer {
    /// Compile the program and allocate the buffer. Call with EGL current.
    pub fn new(viewport: (i32, i32)) -> Renderer {
        unsafe {
            let vs = compile(gl::GL_VERTEX_SHADER, VERTEX_SRC);
            let fs = compile(gl::GL_FRAGMENT_SHADER, FRAGMENT_SRC);
            let program = gl::glCreateProgram();
            gl::glAttachShader(program, vs);
            gl::glAttachShader(program, fs);
            gl::glLinkProgram(program);
            let mut ok = 0;
            gl::glGetProgramiv(program, GL_LINK_STATUS_LOCAL, &mut ok);
            debug_assert_eq!(ok, 1);
            let mut buffer = 0u32;
            gl::glGenBuffers(1, &mut buffer);
            let renderer = Renderer {
                program,
                attrib_pos: gl::glGetAttribLocation(program, b"a_pos\0".as_ptr()),
                attrib_col: gl::glGetAttribLocation(program, b"a_col\0".as_ptr()),
                buffer,
                viewport,
            };
            gl::glViewport(0, 0, viewport.0, viewport.1);
            gl::glClearColor(0.0, 0.0, 0.0, 1.0);
            renderer
        }
    }

    /// Draw one frame's batches.
    pub fn draw(&self, batch: &Batch) {
        unsafe {
            gl::glClear(0x4000); // GL_COLOR_BUFFER_BIT
            gl::glUseProgram(self.program);
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, self.buffer);
            gl::glBufferData(
                gl::GL_ARRAY_BUFFER,
                (batch.data.len() * std::mem::size_of::<f32>()) as isize,
                batch.data.as_ptr(),
                gl::GL_DYNAMIC_DRAW,
            );
            gl::glEnableVertexAttribArray(self.attrib_pos as u32);
            gl::glVertexAttribPointer(
                self.attrib_pos as u32,
                2,
                gl::GL_FLOAT,
                0,
                20,
                std::ptr::null(),
            );
            gl::glEnableVertexAttribArray(self.attrib_col as u32);
            gl::glVertexAttribPointer(
                self.attrib_col as u32,
                3,
                gl::GL_FLOAT,
                0,
                20,
                (2 * std::mem::size_of::<f32>()) as *const (),
            );

            let points_end = batch.points;
            let lines_end = points_end + batch.lines;
            if points_end > 0 {
                gl::glDrawArrays(gl::GL_POINTS, 0, points_end as i32);
            }
            if lines_end > points_end {
                gl::glDrawArrays(gl::GL_LINES, points_end as i32, batch.lines as i32);
            }
            if batch.tris > 0 {
                gl::glDrawArrays(gl::GL_TRIANGLES, lines_end as i32, batch.tris as i32);
            }
        }
    }
}

const GL_LINK_STATUS_LOCAL: u32 = gl::GL_LINK_STATUS;

fn compile(kind: u32, source: &[u8]) -> u32 {
    unsafe {
        let shader = gl::glCreateShader(kind);
        let ptr = source.as_ptr();
        let len = source.len() as i32;
        gl::glShaderSource(shader, 1, &ptr, &len);
        gl::glCompileShader(shader);
        let mut ok = 0;
        gl::glGetShaderiv(shader, gl::GL_COMPILE_STATUS, &mut ok);
        debug_assert_eq!(ok, 1);
        shader
    }
}
