//! The window's GPU path (spec-fråga 66): the display list drawn with
//! OpenGL 3.3 core or OpenGL ES 3.0 through `glow`, on a context the shell
//! makes current (SPEC-9.1.2: only the main process has one).
//!
//! What is drawn comes from the same resolution as the CPU path
//! ([`crate::Renderer`]): the text runs resolved to positioned glyphs, the
//! dry pass's budget report and every op's footprint. Every drawing op
//! becomes one or more instances of one quad, in list order, drawn by one
//! shader (SPEC-6.1.3: later over earlier; the GPU blends in submission
//! order):
//!
//! - **Rectangles**: exact area coverage for a plain one, a signed
//!   distance for a rounded one.
//! - **Clips**: a table of every `CLIP_PUSH` of the frame (a float
//!   texture), each with its parent; an instance names its innermost
//!   clip and the shader multiplies the coverage of the whole chain, as
//!   the reference multiplies masks: a plain clip by the same whole-pixel
//!   rounding as the reference's (`exact_pixels`), a rounded one by its
//!   signed distance.
//! - **Text**: the glyph bitmaps the CPU path rasterized (swash, via
//!   [`crate::TextEngine`]), packed into an R8 atlas once per glyph and
//!   fetched texel for texel at the reference's integer positions.
//! - **Images**: one texture per decoded resource, sampled nearest with
//!   the reference's mapping.
//! - **Paths**: rasterized on the CPU by tiny-skia into coverage, packed
//!   into a per-frame R8 texture.
//!
//! The result is the reference's picture within the tolerance of
//! SPEC-11.3.2 (anti-aliased edges and float blending), never byte for
//! byte; the conformance suite stays on the reference. A frame the
//! pipeline cannot draw ([`GlError::Unsupported`]: a colour glyph, an image
//! larger than the textures, a full atlas) is drawn by the CPU path and
//! uploaded instead.

use crate::image::{ImageCache, RgbaImage};
use crate::raster::{self, Pixels};
use crate::renderer::Resolved;
use cosmic_text::{CacheKey, SwashContent, SwashImage};
use glow::HasContext;
use std::collections::HashMap;
use tiny_skia::{FillRule, Mask, PathBuilder, Stroke, Transform};
use vy_dl::{Op, PathCmd};

/// Why the GL path did not draw.
#[derive(Debug, PartialEq)]
pub enum GlError {
    /// The context cannot run the pipeline (version, shader, link,
    /// framebuffer): the window falls back to the CPU path for good.
    Setup(String),
    /// A GL error after drawing (a lost context, out of memory): the
    /// window falls back to the CPU path for good.
    Draw(String),
    /// This frame needs something the pipeline does not draw; the caller
    /// draws it on the CPU.
    Unsupported(&'static str),
    /// The display list or size is refused, as by the reference.
    Render(crate::RenderError),
}

impl std::fmt::Display for GlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Setup(e) => write!(f, "GL setup failed: {e}"),
            Self::Draw(e) => write!(f, "GL drawing failed: {e}"),
            Self::Unsupported(e) => write!(f, "not drawn by GL: {e}"),
            Self::Render(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GlError {}

/// What the last page drawn on the GPU took: the evidence where timings
/// on a software GL say nothing about a GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlStats {
    /// Quads drawn (one per rectangle, glyph, image and path).
    pub instances: usize,
    /// Draw calls (one, plus one per change of image texture).
    pub draws: usize,
    /// Bytes handed to the GPU: instances, clip table, paths, new glyphs
    /// and new images.
    pub upload_bytes: usize,
    /// Glyph bitmaps added to the atlas.
    pub new_glyphs: usize,
    /// CPU time from the resolved frame to the last GL call issued:
    /// building the instances, the clip table and the paths, and the
    /// calls themselves (not the GPU's work; a software GL does its
    /// vertex work inside the calls).
    pub prepare: std::time::Duration,
    /// Of `prepare`, building the instances, the clip table and the
    /// paths: the CPU work of the pipeline itself, before any GL call.
    pub build: std::time::Duration,
}

/// Bytes per instance: quad (4 × i16, whole pixels), shape (vec4), extra
/// (vec2), colour (RGBA8), kind and clip (2 × u32).
const INSTANCE_BYTES: usize = 44;
/// Side of the glyph atlas.
const ATLAS_SIDE: i32 = 2048;
/// The atlas position of a glyph whose bitmap is shorter than its
/// placement says: the reference draws nothing for it, nor does this.
const NO_BITMAP: (i32, i32) = (-1, -1);
/// Clip-table entries per texture row (two texels each).
const CLIPS_PER_ROW: usize = 1024;
/// Frames an image texture survives unused.
const IMAGE_TTL_FRAMES: u64 = 240;

const KIND_RECT: i32 = 0;
const KIND_RRECT: i32 = 1;
const KIND_GLYPH: i32 = 2;
const KIND_IMAGE: i32 = 3;
const KIND_PATH: i32 = 4;

const VERTEX: &str = r#"
layout(location = 0) in ivec4 a_quad;
layout(location = 1) in vec4 a_shape;
layout(location = 2) in vec2 a_extra;
layout(location = 3) in vec4 a_color;
layout(location = 4) in uvec2 a_misc;
uniform vec2 u_size;
flat out vec4 v_shape;
flat out vec2 v_extra;
flat out vec4 v_color;
flat out ivec2 v_misc;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 q = mix(vec2(a_quad.xy), vec2(a_quad.zw), corner);
    gl_Position = vec4(q.x / u_size.x * 2.0 - 1.0, 1.0 - q.y / u_size.y * 2.0, 0.0, 1.0);
    v_shape = a_shape;
    v_extra = a_extra;
    v_color = a_color;
    v_misc = ivec2(a_misc);
}
"#;

const FRAGMENT: &str = r#"
uniform sampler2D u_atlas;
uniform sampler2D u_paths;
uniform highp sampler2D u_clips;
uniform sampler2D u_image;
uniform float u_height;
flat in vec4 v_shape;
flat in vec2 v_extra;
flat in vec4 v_color;
flat in ivec2 v_misc;
out vec4 o_color;

float rrect(vec2 p, vec4 r, float rad) {
    vec2 c = (r.xy + r.zw) * 0.5;
    vec2 h = (r.zw - r.xy) * 0.5;
    vec2 q = abs(p - c) - h + rad;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rad;
    return clamp(0.5 - d, 0.0, 1.0);
}

float area(vec2 p, vec4 r) {
    vec2 a = clamp(min(p + 0.5, r.zw) - max(p - 0.5, r.xy), 0.0, 1.0);
    return a.x * a.y;
}

float clipped(vec2 p, int k) {
    float cov = 1.0;
    for (int i = 0; i < 40; i++) {
        if (k <= 0) {
            break;
        }
        ivec2 at = ivec2((k % 1024) * 2, k / 1024);
        vec4 r = texelFetch(u_clips, at, 0);
        vec4 e = texelFetch(u_clips, at + ivec2(1, 0), 0);
        if (e.z < 0.5) {
            cov *= (p.x >= r.x && p.x < r.z && p.y >= r.y && p.y < r.w) ? 1.0 : 0.0;
        } else if (e.z < 1.5) {
            cov *= rrect(p, r, e.x);
        } else {
            cov = 0.0;
        }
        k = int(e.y);
    }
    return cov;
}

void main() {
    vec2 p = vec2(gl_FragCoord.x, u_height - gl_FragCoord.y);
    ivec2 px = ivec2(floor(p));
    vec4 c = v_color;
    // The reference's premultiplied uniform colour: (c × a × 255 + 0.5),
    // truncated.
    vec4 pm = vec4(floor(c.rgb * c.a * 255.0 + 0.5) / 255.0, c.a);
    int kind = v_misc.x;
    vec4 o;
    if (kind == 0) {
        o = pm * area(p, v_shape);
    } else if (kind == 1) {
        o = pm * rrect(p, v_shape, v_extra.x);
    } else if (kind == 2) {
        // The reference's tint, in its integers: alpha = (g × a + 127) / 255,
        // each channel (c × alpha + 127) / 255.
        int g = int(texelFetch(u_atlas, ivec2(v_shape.xy) + px - ivec2(v_extra.xy), 0).r * 255.0 + 0.5);
        ivec4 c8 = ivec4(c * 255.0 + 0.5);
        int a = (g * c8.a + 127) / 255;
        ivec3 ch = (c8.rgb * a + 127) / 255;
        o = vec4(vec3(ch), float(a)) / 255.0;
    } else if (kind == 3) {
        ivec2 sz = textureSize(u_image, 0);
        int sx = clamp(int(floor((p.x - v_shape.x) / v_shape.z)), 0, sz.x - 1);
        int sy = clamp(int(floor((p.y - v_shape.y) / v_shape.w)), 0, sz.y - 1);
        ivec4 t = ivec4(texelFetch(u_image, ivec2(sx, sy), 0) * 255.0 + 0.5);
        o = vec4(vec3((t.rgb * t.a + 127) / 255), float(t.a)) / 255.0;
    } else {
        float g = texelFetch(u_paths, ivec2(v_shape.xy) + px - ivec2(v_extra.xy), 0).r;
        o = pm * g;
    }
    o_color = o * clipped(p, v_misc.y);
}
"#;

const BLIT_VERTEX: &str = r#"
uniform vec4 u_rect;
uniform vec2 u_size;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 q = mix(u_rect.xy, u_rect.zw, corner);
    gl_Position = vec4(q.x / u_size.x * 2.0 - 1.0, 1.0 - q.y / u_size.y * 2.0, 0.0, 1.0);
}
"#;

const BLIT_FRAGMENT: &str = r#"
uniform sampler2D u_tex;
uniform vec2 u_origin;
uniform float u_height;
uniform int u_flip;
out vec4 o_color;
void main() {
    vec2 p = vec2(gl_FragCoord.x, u_height - gl_FragCoord.y);
    ivec2 t = ivec2(floor(p - u_origin));
    if (u_flip == 1) {
        t.y = textureSize(u_tex, 0).y - 1 - t.y;
    }
    o_color = texelFetch(u_tex, t, 0);
}
"#;

/// A shelf packer over a `w` × `h` texture.
#[derive(Default)]
struct Shelves {
    w: i32,
    h: i32,
    /// (y, height, next x) of each shelf.
    shelves: Vec<(i32, i32, i32)>,
    bottom: i32,
}

impl Shelves {
    fn new(w: i32, h: i32) -> Self {
        Self {
            w,
            h,
            ..Self::default()
        }
    }

    /// Room for a `w` × `h` box, with a texel of margin.
    fn place(&mut self, w: i32, h: i32) -> Option<(i32, i32)> {
        let (bw, bh) = (w + 1, h + 1);
        if bw > self.w {
            return None;
        }
        for s in &mut self.shelves {
            if bh <= s.1 && s.2 + bw <= self.w {
                let at = (s.2, s.0);
                s.2 += bw;
                return Some(at);
            }
        }
        if self.bottom + bh > self.h {
            return None;
        }
        self.shelves.push((self.bottom, bh, bw));
        let at = (0, self.bottom);
        self.bottom += bh;
        Some(at)
    }
}

/// The glyph atlas: an R8 texture, its CPU copy, where each bitmap is.
struct Atlas {
    tex: glow::Texture,
    shadow: Vec<u8>,
    shelves: Shelves,
    at: HashMap<CacheKey, (i32, i32)>,
    dirty: Option<Pixels>,
    /// Unique across every atlas of the process and new at every clear:
    /// what a run's remembered positions are checked against.
    generation: u64,
}

/// The source of [`Atlas::generation`].
static ATLAS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn atlas_generation() -> u64 {
    ATLAS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Atlas {
    fn clear(&mut self) {
        self.at.clear();
        self.shelves = Shelves::new(ATLAS_SIDE, ATLAS_SIDE);
        self.dirty = None;
        self.generation = atlas_generation();
    }

    /// The atlas positions of every glyph of `run`: remembered on the run
    /// when it was drawn since the atlas last started over, otherwise
    /// looked up (and added) glyph by glyph. `Ok(None)`: the atlas is full.
    fn run(
        &mut self,
        run: &crate::raster::ResolvedRun,
        glyphs: &[SwashImage],
        keys: &[CacheKey],
        stats: &mut GlStats,
    ) -> Result<Option<crate::raster::AtlasSpots>, GlError> {
        let mut memo = run.atlas.lock().unwrap_or_else(|e| e.into_inner());
        if memo.0 == self.generation && memo.1.len() == run.glyphs.len() {
            return Ok(Some(std::sync::Arc::clone(&memo.1)));
        }
        let mut at = Vec::with_capacity(run.glyphs.len());
        for g in &run.glyphs {
            let i = g.image as usize;
            match self.glyph(keys[i], &glyphs[i], stats)? {
                Some(p) => at.push(p),
                None => return Ok(None),
            }
        }
        let at: crate::raster::AtlasSpots = at.into();
        *memo = (self.generation, std::sync::Arc::clone(&at));
        Ok(Some(at))
    }

    /// Where glyph `key` is, adding it; `Err` for a bitmap the pipeline
    /// does not draw, `Ok(None)` when the atlas is full.
    fn glyph(
        &mut self,
        key: CacheKey,
        image: &SwashImage,
        stats: &mut GlStats,
    ) -> Result<Option<(i32, i32)>, GlError> {
        if let Some(&at) = self.at.get(&key) {
            return Ok(Some(at));
        }
        let (w, h) = (image.placement.width as i32, image.placement.height as i32);
        let n = (w * h) as usize;
        let per = match image.content {
            SwashContent::Mask => 1,
            SwashContent::SubpixelMask => 3,
            SwashContent::Color => return Err(GlError::Unsupported("a colour glyph")),
        };
        if image.data.len() < n * per {
            // The reference draws nothing for it either.
            return Ok(Some(NO_BITMAP));
        }
        let Some((x, y)) = self.shelves.place(w, h) else {
            return Ok(None);
        };
        for row in 0..h {
            let dst = ((y + row) * ATLAS_SIDE + x) as usize;
            for col in 0..w {
                let i = (row * w + col) as usize;
                self.shadow[dst + col as usize] = if per == 1 {
                    image.data[i]
                } else {
                    let c = &image.data[i * 3..i * 3 + 3];
                    ((u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) / 3) as u8
                };
            }
        }
        let b = Pixels {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        };
        self.dirty = Some(self.dirty.map_or(b, |d| union(&d, &b)));
        self.at.insert(key, (x, y));
        stats.new_glyphs += 1;
        Ok(Some((x, y)))
    }
}

/// Where the page goes in the window, in physical pixels: its top-left
/// corner, and the rectangle (left, top, width, height) it is cut to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageAt {
    /// Left of the page's canvas.
    pub x: i64,
    /// Top of the page's canvas.
    pub y: i64,
    /// The page area: nothing of the page shows outside it (SPEC-9.6.1).
    pub clip: (i64, i64, i64, i64),
}

/// The page's framebuffer: a texture of the canvas's size.
struct Page {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w: u32,
    h: u32,
}

/// The window's composited frame (chrome and overlays, the page area
/// transparent where the page shows), as a texture.
struct Overlay {
    tex: glow::Texture,
    w: u32,
    h: u32,
}

/// The GL pipeline, one per context (see the module docs).
pub struct GlRaster {
    program: glow::Program,
    blit: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    atlas: Atlas,
    paths: glow::Texture,
    clips: glow::Texture,
    images: HashMap<(u64, u16), (glow::Texture, u64)>,
    page: Option<Page>,
    overlay: Option<Overlay>,
    max_texture: i32,
    frame: u64,
    stats: GlStats,
    version: String,
    instances: Vec<u8>,
}

/// One instance being built.
struct Inst {
    quad: [i16; 4],
    shape: [f32; 4],
    extra: [f32; 2],
    color: u32,
    kind: i32,
    clip: i32,
}

fn push(buf: &mut Vec<u8>, i: &Inst) {
    let mut b = [0u8; INSTANCE_BYTES];
    for (k, v) in i.quad.iter().enumerate() {
        b[k * 2..k * 2 + 2].copy_from_slice(&v.to_ne_bytes());
    }
    for (k, v) in i.shape.iter().chain(&i.extra).enumerate() {
        b[8 + k * 4..12 + k * 4].copy_from_slice(&v.to_ne_bytes());
    }
    b[32..36].copy_from_slice(&i.color.to_be_bytes());
    b[36..40].copy_from_slice(&(i.kind as u32).to_ne_bytes());
    b[40..44].copy_from_slice(&(i.clip as u32).to_ne_bytes());
    buf.extend_from_slice(&b);
}

fn union(a: &Pixels, b: &Pixels) -> Pixels {
    Pixels {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// Whole pixels as an instance's quad. Every op's footprint lies on the
/// canvas, whose side is at most `MAX_CANVAS_SIDE_PX` = 16 384.
fn quad(p: &Pixels) -> [i16; 4] {
    [p.x0 as i16, p.y0 as i16, p.x1 as i16, p.y1 as i16]
}

/// Compile and link a program from the shared sources, with the version
/// line the context speaks.
///
/// # Safety
/// `gl` must be current on this thread.
unsafe fn program(
    gl: &glow::Context,
    header: &str,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program, GlError> {
    let p = gl.create_program().map_err(GlError::Setup)?;
    let mut shaders = Vec::new();
    for (kind, src) in [
        (glow::VERTEX_SHADER, vertex),
        (glow::FRAGMENT_SHADER, fragment),
    ] {
        let sh = gl.create_shader(kind).map_err(GlError::Setup)?;
        gl.shader_source(sh, &format!("{header}{src}"));
        gl.compile_shader(sh);
        if !gl.get_shader_compile_status(sh) {
            return Err(GlError::Setup(format!(
                "shader: {}",
                gl.get_shader_info_log(sh)
            )));
        }
        gl.attach_shader(p, sh);
        shaders.push(sh);
    }
    gl.link_program(p);
    for sh in shaders {
        gl.detach_shader(p, sh);
        gl.delete_shader(sh);
    }
    if !gl.get_program_link_status(p) {
        return Err(GlError::Setup(format!(
            "link: {}",
            gl.get_program_info_log(p)
        )));
    }
    Ok(p)
}

/// A texture with nearest filtering and edge clamping.
///
/// # Safety
/// `gl` must be current on this thread.
unsafe fn texture(gl: &glow::Context) -> Result<glow::Texture, GlError> {
    let t = gl.create_texture().map_err(GlError::Setup)?;
    gl.bind_texture(glow::TEXTURE_2D, Some(t));
    for (k, v) in [
        (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
        (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
    ] {
        gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
    }
    Ok(t)
}

fn gl_error(gl: &glow::Context, what: &str) -> Result<(), GlError> {
    // SAFETY: the callers hold `gl` current.
    let e = unsafe { gl.get_error() };
    if e == glow::NO_ERROR {
        Ok(())
    } else {
        Err(GlError::Draw(format!("{what}: GL error 0x{e:04X}")))
    }
}

impl GlRaster {
    /// Build the pipeline on `gl`, which must be current on this thread
    /// and speak OpenGL 3.3 or later, or OpenGL ES 3.0 or later.
    ///
    /// # Safety
    /// `gl` must be a valid context, current on the calling thread, for
    /// this and every later call with it.
    pub unsafe fn new(gl: &glow::Context) -> Result<Self, GlError> {
        let v = gl.version();
        let ok = if v.is_embedded {
            (v.major, v.minor) >= (3, 0)
        } else {
            (v.major, v.minor) >= (3, 3)
        };
        let version = format!(
            "{} {}.{} ({}; {})",
            if v.is_embedded { "OpenGL ES" } else { "OpenGL" },
            v.major,
            v.minor,
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION)
        );
        if !ok {
            return Err(GlError::Setup(format!(
                "{version}: OpenGL 3.3 or OpenGL ES 3.0 is needed"
            )));
        }
        let header = if v.is_embedded {
            "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp sampler2D;\n"
        } else {
            "#version 330 core\n"
        };
        let program = program(gl, header, VERTEX, FRAGMENT)?;
        let blit = program_blit(gl, header)?;
        let vao = gl.create_vertex_array().map_err(GlError::Setup)?;
        let vbo = gl.create_buffer().map_err(GlError::Setup)?;
        let atlas_tex = texture(gl)?;
        let shadow = vec![0u8; (ATLAS_SIDE * ATLAS_SIDE) as usize];
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::R8 as i32,
            ATLAS_SIDE,
            ATLAS_SIDE,
            0,
            glow::RED,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&shadow)),
        );
        let paths = texture(gl)?;
        let clips = texture(gl)?;
        let max_texture = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE);
        gl_error(gl, "setup")?;
        Ok(Self {
            program,
            blit,
            vao,
            vbo,
            atlas: Atlas {
                tex: atlas_tex,
                shadow,
                shelves: Shelves::new(ATLAS_SIDE, ATLAS_SIDE),
                at: HashMap::new(),
                dirty: None,
                generation: atlas_generation(),
            },
            paths,
            clips,
            images: HashMap::new(),
            page: None,
            overlay: None,
            max_texture,
            frame: 0,
            stats: GlStats::default(),
            version,
            instances: Vec::new(),
        })
    }

    /// The context's API, version and renderer, for the log.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// What the last page drawn here took.
    pub fn stats(&self) -> GlStats {
        self.stats
    }

    /// Draw a resolved frame into the page framebuffer.
    pub(crate) fn draw_page(
        &mut self,
        gl: &glow::Context,
        f: &Resolved,
        glyphs: &[SwashImage],
        keys: &[CacheKey],
        images: &ImageCache,
    ) -> Result<(), GlError> {
        let (w, h) = (f.size.width(), f.size.height());
        if w as i32 > self.max_texture || h as i32 > self.max_texture {
            return Err(GlError::Unsupported("a canvas larger than the textures"));
        }
        self.frame += 1;
        self.stats = GlStats::default();
        let started = std::time::Instant::now();
        let mut built = self.build(gl, f, glyphs, keys, images)?;
        if built.is_none() {
            // The atlas filled up: start it over and try once more.
            self.atlas.clear();
            built = self.build(gl, f, glyphs, keys, images)?;
        }
        let Some((batches, clip_table, path_atlas)) = built else {
            return Err(GlError::Unsupported("more glyphs than the atlas holds"));
        };
        self.stats.build = started.elapsed();
        // SAFETY: `gl` is current (the contract of `new`), and every
        // slice handed over is as long as the size given with it.
        unsafe {
            self.ensure_page(gl, w, h)?;
            self.upload_atlas(gl);
            self.upload_clips(gl, &clip_table);
            self.upload_paths(gl, &path_atlas);
            let page = self.page.as_ref().expect("made above");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(page.fbo));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.clear_color(1.0, 1.0, 1.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.use_program(Some(self.program));
            let loc = |n: &str| gl.get_uniform_location(self.program, n);
            gl.uniform_2_f32(loc("u_size").as_ref(), w as f32, h as f32);
            gl.uniform_1_f32(loc("u_height").as_ref(), h as f32);
            for (unit, name, tex) in [
                (0, "u_atlas", self.atlas.tex),
                (1, "u_paths", self.paths),
                (2, "u_clips", self.clips),
            ] {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.uniform_1_i32(loc(name).as_ref(), unit as i32);
            }
            gl.uniform_1_i32(loc("u_image").as_ref(), 3);
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &self.instances, glow::STREAM_DRAW);
            self.stats.upload_bytes += self.instances.len();
            self.stats.instances = self.instances.len() / INSTANCE_BYTES;
            for (start, count, image) in batches {
                gl.active_texture(glow::TEXTURE3);
                gl.bind_texture(glow::TEXTURE_2D, image);
                let base = (start * INSTANCE_BYTES) as i32;
                let stride = INSTANCE_BYTES as i32;
                for loc in 0..5 {
                    gl.enable_vertex_attrib_array(loc);
                    gl.vertex_attrib_divisor(loc, 1);
                }
                gl.vertex_attrib_pointer_i32(0, 4, glow::SHORT, stride, base);
                gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, base + 8);
                gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, base + 24);
                gl.vertex_attrib_pointer_f32(3, 4, glow::UNSIGNED_BYTE, true, stride, base + 32);
                gl.vertex_attrib_pointer_i32(4, 2, glow::UNSIGNED_INT, stride, base + 36);
                gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, count as i32);
                self.stats.draws += 1;
            }
            gl.bind_vertex_array(None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.stats.prepare = started.elapsed();
        gl_error(gl, "page")?;
        // Images not drawn for a while go.
        let frame = self.frame;
        let stale: Vec<_> = self
            .images
            .iter()
            .filter(|(_, (_, used))| frame - used > IMAGE_TTL_FRAMES)
            .map(|(k, (t, _))| (*k, *t))
            .collect();
        for (k, t) in stale {
            // SAFETY: as above.
            unsafe { gl.delete_texture(t) };
            self.images.remove(&k);
        }
        Ok(())
    }

    /// The instances of a frame, in batches of one image texture each,
    /// with the clip table and the path coverage; `None` when the atlas
    /// is full.
    #[allow(clippy::type_complexity)]
    fn build(
        &mut self,
        gl: &glow::Context,
        f: &Resolved,
        glyphs: &[SwashImage],
        keys: &[CacheKey],
        images: &ImageCache,
    ) -> Result<Option<(Vec<Batch>, Vec<f32>, PathAtlas)>, GlError> {
        let s = f.scale;
        let (cw, ch) = (f.size.width(), f.size.height());
        self.instances.clear();
        let mut batches: Vec<Batch> = Vec::new();
        let mut clip_table = vec![0.0f32; 8];
        let mut stack: Vec<i32> = Vec::new();
        let mut paths = PathAtlas::new(self.max_texture.min(2048));
        let mut current: Option<glow::Texture> = None;
        let mut start = 0usize;
        let mut count = 0usize;
        for (i, op) in f.ops[..f.report.ops_done].iter().enumerate() {
            let clip = stack.last().copied().unwrap_or(0);
            match op {
                Op::ClipPush { x, y, w, h, radius } => {
                    let parent = clip as f32;
                    let mut e = [0.0f32; 8];
                    e[5] = parent;
                    let rect = raster::sane(&[*x, *y, *w, *h, *radius])
                        .then(|| {
                            tiny_skia::Rect::from_xywh(
                                x * s,
                                y * s,
                                (w * s).max(0.0),
                                (h * s).max(0.0),
                            )
                        })
                        .flatten();
                    match rect {
                        Some(r) if radius * s > 0.0 => {
                            let rad = (radius * s).min(r.width() / 2.0).min(r.height() / 2.0);
                            e[..4].copy_from_slice(&[r.left(), r.top(), r.right(), r.bottom()]);
                            e[4] = rad;
                            e[6] = 1.0;
                        }
                        Some(r) => match raster::exact_pixels(&r, cw, ch) {
                            Some(p) => e[..4].copy_from_slice(&[
                                p.x0 as f32,
                                p.y0 as f32,
                                p.x1 as f32,
                                p.y1 as f32,
                            ]),
                            None => e[6] = 2.0,
                        },
                        None => e[6] = 2.0,
                    }
                    clip_table.extend_from_slice(&e);
                    stack.push((clip_table.len() / 8 - 1) as i32);
                }
                Op::ClipPop => {
                    stack.pop();
                }
                _ => {
                    let Some(fp) = f.footprints[i] else {
                        continue;
                    };
                    let mut image_tex = None;
                    let before = self.instances.len();
                    match op {
                        Op::Rect {
                            x,
                            y,
                            w,
                            h,
                            radius,
                            color,
                        } => {
                            let r = [x * s, y * s, (x + w) * s, (y + h) * s];
                            let rad = (radius * s).max(0.0).min((w * s) / 2.0).min((h * s) / 2.0);
                            let rounded = *radius > 0.0 && rad > 0.0;
                            push(
                                &mut self.instances,
                                &Inst {
                                    quad: quad(&fp),
                                    shape: r,
                                    extra: [rad, 0.0],
                                    color: *color,
                                    kind: if rounded { KIND_RRECT } else { KIND_RECT },
                                    clip,
                                },
                            );
                        }
                        Op::Image {
                            x,
                            y,
                            w,
                            h,
                            resource_id,
                            fit,
                        } => match images.get(*resource_id) {
                            Err(_) => push(
                                &mut self.instances,
                                &Inst {
                                    quad: quad(&fp),
                                    shape: [x * s, y * s, (x + w) * s, (y + h) * s],
                                    extra: [0.0; 2],
                                    color: raster::IMAGE_PLACEHOLDER_COLOR,
                                    kind: KIND_RECT,
                                    clip,
                                },
                            ),
                            Ok(img) => {
                                let Some(p) = raster::place(
                                    *fit,
                                    img.width,
                                    img.height,
                                    x * s,
                                    y * s,
                                    w * s,
                                    h * s,
                                    cw,
                                    ch,
                                ) else {
                                    continue;
                                };
                                let all = Pixels {
                                    x0: p.px0,
                                    y0: p.py0,
                                    x1: p.px1,
                                    y1: p.py1,
                                };
                                let Some(q) = all.intersect(&fp) else {
                                    continue;
                                };
                                image_tex = Some(self.image(gl, images, *resource_id, img)?);
                                push(
                                    &mut self.instances,
                                    &Inst {
                                        quad: quad(&q),
                                        shape: [p.rx, p.ry, p.kx, p.ky],
                                        extra: [0.0; 2],
                                        color: 0,
                                        kind: KIND_IMAGE,
                                        clip,
                                    },
                                );
                            }
                        },
                        Op::Text { .. } => {
                            let Some(Some(t)) = f.text_ops.get(i) else {
                                continue;
                            };
                            let Some(at) = self.atlas.run(&t.run, glyphs, keys, &mut self.stats)?
                            else {
                                return Ok(None);
                            };
                            for (g, &(ax, ay)) in t.run.glyphs.iter().zip(at.iter()) {
                                let img = &glyphs[g.image as usize];
                                let gb = Pixels {
                                    x0: g.x,
                                    y0: g.y,
                                    x1: g.x.saturating_add(img.placement.width as i32),
                                    y1: g.y.saturating_add(img.placement.height as i32),
                                };
                                let Some(q) = gb.intersect(&fp) else {
                                    continue;
                                };
                                if (ax, ay) == NO_BITMAP {
                                    continue;
                                }
                                push(
                                    &mut self.instances,
                                    &Inst {
                                        quad: quad(&q),
                                        shape: [ax as f32, ay as f32, 0.0, 0.0],
                                        extra: [g.x as f32, g.y as f32],
                                        color: t.color,
                                        kind: KIND_GLYPH,
                                        clip,
                                    },
                                );
                            }
                        }
                        Op::Path {
                            color,
                            stroke_w,
                            cmds,
                        } => {
                            let Some((ax, ay)) = paths.add(cmds, *stroke_w, s, &fp)? else {
                                continue;
                            };
                            push(
                                &mut self.instances,
                                &Inst {
                                    quad: quad(&fp),
                                    shape: [ax as f32, ay as f32, 0.0, 0.0],
                                    extra: [fp.x0 as f32, fp.y0 as f32],
                                    color: *color,
                                    kind: KIND_PATH,
                                    clip,
                                },
                            );
                        }
                        Op::ClipPush { .. } | Op::ClipPop => unreachable!("handled above"),
                    }
                    let added = (self.instances.len() - before) / INSTANCE_BYTES;
                    if let Some(tex) = image_tex {
                        if current.is_some_and(|c| c != tex) {
                            // A new image texture: the instances so far are
                            // one draw.
                            batches.push((start, count, current));
                            start += count;
                            count = 0;
                        }
                        current = Some(tex);
                    }
                    count += added;
                }
            }
        }
        batches.push((start, count, current));
        Ok(Some((batches, clip_table, paths)))
    }

    /// The texture of decoded image `id`, uploaded on first use.
    fn image(
        &mut self,
        gl: &glow::Context,
        images: &ImageCache,
        id: u16,
        img: &RgbaImage,
    ) -> Result<glow::Texture, GlError> {
        let key = (images.id(), id);
        let frame = self.frame;
        if let Some((t, used)) = self.images.get_mut(&key) {
            *used = frame;
            return Ok(*t);
        }
        if img.width as i32 > self.max_texture || img.height as i32 > self.max_texture {
            return Err(GlError::Unsupported("an image larger than the textures"));
        }
        // SAFETY: `gl` is current (the contract of `new`); the data is
        // width × height × 4 bytes (`RgbaImage`).
        let t = unsafe {
            let t = texture(gl)?;
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                img.width as i32,
                img.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&img.data)),
            );
            t
        };
        self.stats.upload_bytes += img.data.len();
        self.images.insert(key, (t, frame));
        Ok(t)
    }

    /// The page framebuffer at `w` × `h`, made or remade.
    ///
    /// # Safety
    /// `gl` must be current on this thread.
    unsafe fn ensure_page(&mut self, gl: &glow::Context, w: u32, h: u32) -> Result<(), GlError> {
        if self.page.as_ref().is_some_and(|p| (p.w, p.h) == (w, h)) {
            return Ok(());
        }
        if let Some(p) = self.page.take() {
            gl.delete_framebuffer(p.fbo);
            gl.delete_texture(p.tex);
        }
        let tex = texture(gl)?;
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w as i32,
            h as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let fbo = gl.create_framebuffer().map_err(GlError::Setup)?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if status != glow::FRAMEBUFFER_COMPLETE {
            return Err(GlError::Setup(format!("framebuffer 0x{status:04X}")));
        }
        self.page = Some(Page { fbo, tex, w, h });
        Ok(())
    }

    /// Send the atlas rows new glyphs went to.
    ///
    /// # Safety
    /// `gl` must be current on this thread.
    unsafe fn upload_atlas(&mut self, gl: &glow::Context) {
        let Some(d) = self.atlas.dirty.take() else {
            return;
        };
        gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas.tex));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
        let rows = &self.atlas.shadow[(d.y0 * ATLAS_SIDE) as usize..(d.y1 * ATLAS_SIDE) as usize];
        gl.tex_sub_image_2d(
            glow::TEXTURE_2D,
            0,
            0,
            d.y0,
            ATLAS_SIDE,
            d.y1 - d.y0,
            glow::RED,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(rows)),
        );
        self.stats.upload_bytes += rows.len();
    }

    /// Send the clip table: two RGBA32F texels per entry.
    ///
    /// # Safety
    /// `gl` must be current on this thread.
    unsafe fn upload_clips(&mut self, gl: &glow::Context, table: &[f32]) {
        let entries = table.len() / 8;
        let rows = entries.div_ceil(CLIPS_PER_ROW).max(1);
        let mut data = table.to_vec();
        data.resize(rows * CLIPS_PER_ROW * 8, 0.0);
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
        gl.bind_texture(glow::TEXTURE_2D, Some(self.clips));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA32F as i32,
            (CLIPS_PER_ROW * 2) as i32,
            rows as i32,
            0,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelUnpackData::Slice(Some(&bytes)),
        );
        self.stats.upload_bytes += bytes.len();
    }

    /// Send the frame's path coverage.
    ///
    /// # Safety
    /// `gl` must be current on this thread.
    unsafe fn upload_paths(&mut self, gl: &glow::Context, paths: &PathAtlas) {
        let h = paths.used.max(1);
        let mut data = paths.data.clone();
        data.resize((h * paths.w) as usize, 0);
        gl.bind_texture(glow::TEXTURE_2D, Some(self.paths));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::R8 as i32,
            paths.w,
            h,
            0,
            glow::RED,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&data)),
        );
        self.stats.upload_bytes += data.len();
    }

    /// The page as drawn, read back (for the equivalence tests): `None`
    /// before the first page.
    pub fn read_page(&self, gl: &glow::Context) -> Option<RgbaImage> {
        let page = self.page.as_ref()?;
        let (w, h) = (page.w as usize, page.h as usize);
        let mut data = vec![0u8; w * h * 4];
        // SAFETY: `gl` is current; `data` holds w × h RGBA texels.
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(page.fbo));
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut data)),
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        }
        // GL rows run bottom up; the canvas's top down.
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            out[y * w * 4..(y + 1) * w * 4]
                .copy_from_slice(&data[(h - 1 - y) * w * 4..(h - y) * w * 4]);
        }
        Some(RgbaImage {
            width: page.w,
            height: page.h,
            data: out,
        })
    }

    /// Put a frame drawn on the CPU into the page framebuffer (a frame
    /// the pipeline does not draw, [`GlError::Unsupported`]).
    pub fn upload_page(&mut self, gl: &glow::Context, img: &RgbaImage) -> Result<(), GlError> {
        // SAFETY: `gl` is current; the rows are reversed into GL order
        // and are width × 4 bytes each.
        unsafe {
            self.ensure_page(gl, img.width, img.height)?;
            let page = self.page.as_ref().expect("made above");
            let row = img.width as usize * 4;
            let mut flipped = vec![0u8; img.data.len()];
            for y in 0..img.height as usize {
                let src = (img.height as usize - 1 - y) * row;
                flipped[y * row..(y + 1) * row].copy_from_slice(&img.data[src..src + row]);
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(page.tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                img.width as i32,
                img.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&flipped)),
            );
        }
        gl_error(gl, "page upload")
    }

    /// Present on the default framebuffer, `width` × `height`: the page
    /// at `page_at` (physical pixels, top-left), cut to `clip` (left,
    /// top, width, height), then `frame` over everything with its alpha
    /// (the shell's frame, transparent where the page shows). Only the
    /// rows `changed` (top-down, `y0..y1`) of `frame` are sent again; a new
    /// size sends all of it.
    pub fn present(
        &mut self,
        gl: &glow::Context,
        frame: &RgbaImage,
        changed: Option<(u32, u32)>,
        page_at: Option<PageAt>,
    ) -> Result<(), GlError> {
        let (w, h) = (frame.width, frame.height);
        // SAFETY: `gl` is current; every upload slice is a whole number
        // of frame rows.
        unsafe {
            let fresh = !self.overlay.as_ref().is_some_and(|o| (o.w, o.h) == (w, h));
            if fresh {
                if let Some(o) = self.overlay.take() {
                    gl.delete_texture(o.tex);
                }
                let tex = texture(gl)?;
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    w as i32,
                    h as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&frame.data)),
                );
                self.overlay = Some(Overlay { tex, w, h });
            } else if let (Some((y0, y1)), Some(o)) = (changed, &self.overlay) {
                let row = w as usize * 4;
                gl.bind_texture(glow::TEXTURE_2D, Some(o.tex));
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    y0 as i32,
                    w as i32,
                    (y1 - y0) as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(
                        &frame.data[y0 as usize * row..y1 as usize * row],
                    )),
                );
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(1.0, 1.0, 1.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.blit));
            let loc = |n: &str| gl.get_uniform_location(self.blit, n);
            gl.uniform_2_f32(loc("u_size").as_ref(), w as f32, h as f32);
            gl.uniform_1_f32(loc("u_height").as_ref(), h as f32);
            gl.uniform_1_i32(loc("u_tex").as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(self.vao));
            for loc in 0..5 {
                gl.disable_vertex_attrib_array(loc);
            }
            if let (
                Some(PageAt {
                    x: px,
                    y: py,
                    clip: (cx, cy, cw, ch),
                }),
                Some(page),
            ) = (page_at, &self.page)
            {
                gl.disable(glow::BLEND);
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(
                    cx as i32,
                    h as i32 - (cy + ch) as i32,
                    cw.max(0) as i32,
                    ch.max(0) as i32,
                );
                gl.bind_texture(glow::TEXTURE_2D, Some(page.tex));
                gl.uniform_4_f32(
                    loc("u_rect").as_ref(),
                    px as f32,
                    py as f32,
                    (px + i64::from(page.w)) as f32,
                    (py + i64::from(page.h)) as f32,
                );
                gl.uniform_2_f32(loc("u_origin").as_ref(), px as f32, py as f32);
                gl.uniform_1_i32(loc("u_flip").as_ref(), 1);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.disable(glow::SCISSOR_TEST);
            }
            let o = self.overlay.as_ref().expect("made above");
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_texture(glow::TEXTURE_2D, Some(o.tex));
            gl.uniform_4_f32(loc("u_rect").as_ref(), 0.0, 0.0, w as f32, h as f32);
            gl.uniform_2_f32(loc("u_origin").as_ref(), 0.0, 0.0);
            gl.uniform_1_i32(loc("u_flip").as_ref(), 0);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
        }
        gl_error(gl, "present")
    }
}

/// `(first instance, instances, image texture)` of one draw call.
type Batch = (usize, usize, Option<glow::Texture>);

/// The coverage of the frame's paths, packed into one R8 texture.
struct PathAtlas {
    shelves: Shelves,
    data: Vec<u8>,
    w: i32,
    used: i32,
}

impl PathAtlas {
    fn new(w: i32) -> Self {
        Self {
            shelves: Shelves::new(w, i32::MAX / 2),
            data: Vec::new(),
            w,
            used: 0,
        }
    }

    /// Rasterize a path's coverage over its footprint `fp` with tiny-skia,
    /// as the reference builds it (a fill, or the stroke's outline
    /// filled), and place it; `None` when nothing is covered.
    fn add(
        &mut self,
        cmds: &[PathCmd],
        stroke_w: f32,
        s: f32,
        fp: &Pixels,
    ) -> Result<Option<(i32, i32)>, GlError> {
        let (w, h) = (fp.x1 - fp.x0, fp.y1 - fp.y0);
        if w > self.w {
            return Err(GlError::Unsupported("a path wider than the textures"));
        }
        let mut pb = PathBuilder::new();
        for c in cmds {
            match *c {
                PathCmd::MoveTo(x, y) => pb.move_to(x, y),
                PathCmd::LineTo(x, y) => pb.line_to(x, y),
                PathCmd::QuadTo { ctrl, to } => pb.quad_to(ctrl.0, ctrl.1, to.0, to.1),
                PathCmd::Close => pb.close(),
            }
        }
        let Some(path) = pb.finish() else {
            return Ok(None);
        };
        let to_mask = Transform::from_scale(s, s).post_translate(-fp.x0 as f32, -fp.y0 as f32);
        let outline = if stroke_w > 0.0 {
            let stroke = Stroke {
                width: stroke_w,
                ..Stroke::default()
            };
            path.stroke(&stroke, s)
        } else {
            Some(path)
        };
        let Some(outline) = outline.and_then(|p| p.transform(to_mask)) else {
            return Ok(None);
        };
        let Some(mut mask) = Mask::new(w as u32, h as u32) else {
            return Ok(None);
        };
        mask.fill_path(&outline, FillRule::Winding, true, Transform::identity());
        let Some((x, y)) = self.shelves.place(w, h) else {
            return Err(GlError::Unsupported("too many paths"));
        };
        let need = ((y + h + 1) * self.w) as usize;
        if self.data.len() < need {
            self.data.resize(need, 0);
        }
        for row in 0..h {
            let src = &mask.data()[(row * w) as usize..((row + 1) * w) as usize];
            let at = ((y + row) * self.w + x) as usize;
            self.data[at..at + w as usize].copy_from_slice(src);
        }
        self.used = self.used.max(y + h + 1);
        Ok(Some((x, y)))
    }
}

/// The blit program, for the page and the window's frame.
///
/// # Safety
/// `gl` must be current on this thread.
unsafe fn program_blit(gl: &glow::Context, header: &str) -> Result<glow::Program, GlError> {
    program(gl, header, BLIT_VERTEX, BLIT_FRAGMENT)
}
