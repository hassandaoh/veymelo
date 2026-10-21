//! What the GPU draws, laid out as its kernels read it: each struct here is the one of the same
//! name in the kernel's source, field for field (only 16-byte fields, so both lay them out alike).

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use veymelo_os::gpu::{Buffer, Kernel, Plain, Shapes};

/// A run of 3D layers put together (three.metal), and the same with models casting shadows
/// (traced through their triangles).
pub(crate) const SCENE: Kernel = Kernel { source: concat!(include_str!("light.metal"), include_str!("shadows.metal"), include_str!("three.metal")), name: "scene" };
pub(crate) const SCENE_RAYS: Kernel = Kernel { source: concat!("#define RAYS 1\n", include_str!("light.metal"), include_str!("shadows.metal"), include_str!("three.metal")), name: "scene" };

/// A 3D model filled (meshes.metal), and the same with models casting shadows on it.
pub(crate) const MESH: Kernel = Kernel { source: concat!(include_str!("light.metal"), include_str!("shadows.metal"), include_str!("meshes.metal")), name: "mesh" };
pub(crate) const MESH_RAYS: Kernel = Kernel { source: concat!("#define RAYS 1\n", include_str!("light.metal"), include_str!("shadows.metal"), include_str!("meshes.metal")), name: "mesh" };

/// The light bouncing onto the points it is gathered at (shadows.metal), and the same with
/// models to meet.
pub(crate) const GATHER: Kernel = Kernel { source: concat!(include_str!("light.metal"), include_str!("shadows.metal")), name: "gather" };
pub(crate) const GATHER_RAYS: Kernel = Kernel { source: concat!("#define RAYS 1\n", include_str!("light.metal"), include_str!("shadows.metal")), name: "gather" };

/// The pixels a tile of a model's triangles covers, across and down (as meshes.metal has them).
pub(crate) const TILE: [usize; 2] = [32, 8];

/// The most pixels of a run the GPU leaves to the processor (those with more pieces than it keeps
/// at a pixel): past them, the processor draws the run.
pub(crate) const MOST_LEFT: usize = 1 << 16;

/// An environment's panoramas for a kernel (light.metal's `Env`): its pixels and its levels, its
/// turn and how many it shines with (none: two empty buffers, 0 and 0).
pub(crate) fn environment(env: Option<&crate::scene::Environment>) -> Result<(Buffer, Buffer, f32, u32), String> {
    match env {
        Some(e) => {
            let (px, levels) = e.packed();
            Ok((take_with(ENV, px)?, take_with(ENV_LEVELS, levels)?, e.rotation() as f32, e.shining() as u32))
        }
        None => Ok((take(ENV, 16)?, take(ENV_LEVELS, 16)?, 0.0, 0)),
    }
}

/// A scene's flat layers and what casts shadows, as the GPU reads them (shadows.metal's
/// `Shadows`): every flat layer as rays meet it, their own pictures one after another (each
/// one's first texel in `at`), and the triangles of the models casting shadows arranged for rays.
pub(crate) struct Shadows {
    pub casters: Buffer,
    pub count: u32,
    pub texels: Buffer,
    pub at: Vec<u32>,
    pub models: Option<Shapes>,
    /// Every model's triangles (for the ambient occlusion and the reflections), and each as a
    /// reflection sees it (shadows.metal's `Face`).
    pub everything: Option<Shapes>,
    pub faces: Buffer,
    /// Whether any casting model is glass, then each casting triangle's face (`NONE`: not glass).
    pub glass: Buffer,
    /// The pictures the faces read (none: none does).
    pub pictures: Option<std::sync::Arc<crate::meshes::FacePictures>>,
    /// In a scene of many flat layers, a triangle over each one's picture for rays to find them
    /// by, and whether they do (none: rays look at every one).
    pub flats: Option<Shapes>,
    pub flat_use: Buffer,
}

/// A frame's worth of pictures is copied by all the cores, a piece each.
pub(crate) const PIECE: usize = 1 << 16;

impl Shadows {
    /// Nothing casting shadows.
    pub(crate) fn none() -> Result<Shadows, String> {
        Ok(Shadows { casters: Buffer::new(16)?, count: 0, texels: Buffer::new(16)?, at: Vec::new(), models: None, everything: None, faces: Buffer::new(16)?, glass: Buffer::new(16)?, pictures: None, flats: None, flat_use: Buffer::new(16)? })
    }

    /// Flat layers `planes` (each with its own picture), the triangles of models casting
    /// shadows and those of every model hiding the sky (three corners each, in the world), each
    /// as a reflection sees it, and the pictures they read, and (in a scene of many flat layers)
    /// a triangle over each layer's picture: none when there are models to trace and the GPU
    /// cannot, or the pictures are too many.
    pub(crate) fn new(planes: &[(Caster, &veymelo_raster::Image)], models: &[[f32; 3]], everything: &[[f32; 3]], faces: &[crate::meshes::Face], glass: &[u32], pictures: Option<std::sync::Arc<crate::meshes::FacePictures>>, flats: &[[f32; 3]]) -> Result<Option<Shadows>, String> {
        if (!models.is_empty() || !everything.is_empty()) && !veymelo_os::gpu::traces() {
            return Ok(None);
        }
        let (models, everything) = (Shapes::new(models)?, Shapes::new(everything)?);
        let flats = if veymelo_os::gpu::traces() { Shapes::new(flats)? } else { None };
        let flat_use = Buffer::of(&[flats.is_some() as u32])?;
        let total: usize = planes.iter().map(|(_, img)| img.px.len()).sum();
        if total > u32::MAX as usize || total * 16 > 1 << 31 {
            return Ok(None);
        }
        let mut texels = take(PICTURES, total * 16)?;
        let (mut list, mut at) = (Vec::with_capacity(planes.len()), Vec::with_capacity(planes.len()));
        // the pictures one after another: those the GPU made (in memory it shares) copied by
        // the GPU, the rest by the cores
        let mut on_gpu = Vec::new();
        {
            let mut out = &mut texels.slice_mut::<[f32; 4]>()[..total];
            let mut first = 0;
            for (c, img) in planes {
                let (to, rest) = out.split_at_mut(img.px.len());
                match img.px.buffer() {
                    Some(b) => on_gpu.push((b, first * 16, img.px.len() * 16)),
                    None => veymelo_raster::par::chunks(to, PIECE, |k, c| c.copy_from_slice(&img.px[k * PIECE..k * PIECE + c.len()])),
                }
                out = rest;
                list.push(Caster { img: [first as u32, img.width as u32, img.height as u32, c.img[3]], ..*c });
                at.push(first as u32);
                first += img.px.len();
            }
        }
        let copies: Vec<_> = on_gpu.iter().map(|&(b, into, size)| (b, 0, &texels, into, size)).collect();
        veymelo_os::gpu::copy_between(&copies)?;
        Ok(Some(Shadows { casters: take_with(CASTERS, &list)?, count: list.len() as u32, texels, at, models, everything, faces: take_with(FACES, faces)?, glass: take_with(GLASS, glass)?, pictures, flats, flat_use }))
    }

    /// The kernel to run (`plain`, or `rays` when there are models to trace), and what it takes
    /// after its other arguments: both arrangements of triangles (one there is not: one far off),
    /// the glass, and the faces' pictures (where they are, their texels).
    pub(crate) fn kernel<'s>(&'s self, plain: Kernel, rays: Kernel) -> Result<(Kernel, Vec<veymelo_os::gpu::Arg<'s>>), String> {
        use veymelo_os::gpu::Arg;
        if self.models.is_none() && self.everything.is_none() && self.flats.is_none() {
            return Ok((plain, Vec::new()));
        }
        let none = nowhere()?;
        let blank = blank()?;
        let (pics, texels) = self.pictures.as_ref().map_or((blank, blank), |p| (&p.pics, &p.texels));
        Ok((rays, vec![Arg::Shapes(self.models.as_ref().unwrap_or(none)), Arg::Shapes(self.everything.as_ref().unwrap_or(none)), Arg::Read(&self.glass), Arg::Read(pics), Arg::Read(texels), Arg::Shapes(self.flats.as_ref().unwrap_or(none)), Arg::Read(&self.flat_use)]))
    }

    /// Its buffers back, for the next frame drawn on this thread.
    pub(crate) fn keep(self) {
        keep(CASTERS, self.casters);
        keep(PICTURES, self.texels);
        keep(FACES, self.faces);
        keep(GLASS, self.glass);
    }
}

/// A buffer with nothing in it, for a kernel to read where there is nothing to.
fn blank() -> Result<&'static Buffer, String> {
    static BLANK: std::sync::OnceLock<Buffer> = std::sync::OnceLock::new();
    if BLANK.get().is_none() {
        let _ = BLANK.set(Buffer::new(16)?);
    }
    BLANK.get().ok_or_else(|| "the GPU made no buffer".to_string())
}

/// A tiny triangle far off, for a kernel to trace through where there is nothing to.
fn nowhere() -> Result<&'static Shapes, String> {
    static NOWHERE: std::sync::OnceLock<Option<Shapes>> = std::sync::OnceLock::new();
    if NOWHERE.get().is_none() {
        let far = 1.0e9f32;
        let made = Shapes::new(&[[far, far, far], [far + 1.0, far, far], [far, far + 1.0, far]])?;
        let _ = NOWHERE.set(made);
    }
    NOWHERE.get().and_then(Option::as_ref).ok_or_else(|| "the GPU made no triangles".to_string())
}

/// What the kernel takes for "no matte".
pub(crate) const NONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Plane {
    pub m: [f32; 4],
    pub w: [f32; 4],
    pub at: [f32; 4],
    pub dz: [f32; 4],
    pub tex: [f32; 4],
    pub reach: [i32; 4],
    pub img: [u32; 4],
    pub origin: [f32; 4],
    pub across: [f32; 4],
    pub down: [f32; 4],
    pub normal: [f32; 4],
    pub mat0: [f32; 4],
    pub mat1: [f32; 4],
    pub flags: [u32; 4],
    pub more: [u32; 4],
    pub order: [u32; 4],
    pub mirror: [f32; 4],
    pub map: [u32; 4],
    pub cell: [f32; 4],
    pub caustic: [u32; 4],
    pub bent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Light {
    pub at: [f32; 4],
    pub dir: [f32; 4],
    pub color: [f32; 4],
    pub fall: [f32; 4],
    pub kind: [u32; 4],
    pub more: [f32; 4],
}

impl Light {
    pub(crate) fn of(l: &crate::scene::Light) -> Light {
        let f = |v: f64| v as f32;
        Light {
            at: [f(l.at.x), f(l.at.y), f(l.at.z), f(l.cone)],
            dir: [f(l.dir.x), f(l.dir.y), f(l.dir.z), f(l.feather)],
            color: [l.color[0], l.color[1], l.color[2], f(l.radius)],
            fall: [f(l.falloff_distance), f(l.darkness), f(l.diffusion), 0.0],
            kind: [
                match l.kind {
                    veymelo_project::LightKind::Parallel => 0,
                    veymelo_project::LightKind::Spot => 1,
                    veymelo_project::LightKind::Ambient => 3,
                    _ => 2,
                },
                l.falloff,
                l.shadows as u32,
                0,
            ],
            more: [l.occlusion, f(l.occlusion_distance), 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Params {
    pub a: [u32; 4],
    pub b: [u32; 4],
    pub eye: [f32; 4],
    pub c: [u32; 4],
    pub d: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Caster {
    pub origin: [f32; 4],
    pub normal: [f32; 4],
    pub unworld: [[f32; 4]; 4],
    pub own: [f32; 4],
    pub img: [u32; 4],
    pub mat: [f32; 4],
    pub map: [u32; 4],
    pub cell: [f32; 4],
    pub mirror: [f32; 4],
    pub caustic: [u32; 4],
    pub bent: [f32; 4],
}

impl Caster {
    /// The grid of points the light bouncing onto a plane is gathered at, as the kernels take
    /// it (`map`, `cell`): none, a grid with nothing in it.
    pub(crate) fn grid(g: Option<crate::bounce::Grid>) -> ([u32; 4], [f32; 4]) {
        match g {
            Some(g) => ([g.first as u32, g.nx as u32, g.ny as u32, 1], [g.u0 as f32, g.v0 as f32, g.du as f32, g.dv as f32]),
            None => ([0; 4], [0.0, 0.0, 1.0, 1.0]),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Probe {
    pub at: [f32; 4],
    pub n: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct BounceParams {
    pub a: [u32; 4],
    pub b: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Shot {
    pub reach: [i32; 4],
    pub at: [u32; 4],
    pub how: [f32; 4],
    pub more: [u32; 4],
    pub order: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Vert {
    pub screen: [f32; 4],
    pub world: [f32; 4],
    pub normal: [f32; 4],
    pub tangent: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub bounce: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Tri {
    pub v: [u32; 4],
    pub reach: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Mat {
    pub base: [f32; 4],
    pub emissive: [f32; 4],
    pub pbr: [f32; 4],
    pub maps: [u32; 4],
    pub more: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Pic {
    pub a: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct MeshParams {
    pub a: [u32; 4],
    pub b: [u32; 4],
    pub eye: [f32; 4],
    pub layer: [f32; 4],
    pub c: [u32; 4],
    pub d: [f32; 4],
    pub e: [f32; 4],
    pub f: [f32; 4],
}

unsafe impl Plain for Plane {}
unsafe impl Plain for Light {}
unsafe impl Plain for Params {}
unsafe impl Plain for Shot {}
unsafe impl Plain for Caster {}
unsafe impl Plain for Probe {}
unsafe impl Plain for BounceParams {}
unsafe impl Plain for crate::meshes::Face {}
unsafe impl Plain for Vert {}
unsafe impl Plain for Tri {}
unsafe impl Plain for Mat {}
unsafe impl Plain for Pic {}
unsafe impl Plain for MeshParams {}

static DRAWN: AtomicU64 = AtomicU64::new(0);

/// How many runs of 3D layers the GPU has put together in this process.
pub fn scenes_drawn() -> u64 {
    DRAWN.load(Ordering::Relaxed)
}

pub(crate) fn drew_scene() {
    DRAWN.fetch_add(1, Ordering::Relaxed);
}

/// The buffers a drawing thread keeps from one frame to the next, by what they hold (each made
/// again only when a bigger one is needed, so the GPU's memory is not asked for every frame).
pub(crate) const TEXELS: usize = 0;
pub(crate) const ROWS: usize = 1;
pub(crate) const PLANES: usize = 2;
pub(crate) const MATTES: usize = 3;
pub(crate) const LIGHTS: usize = 4;
pub(crate) const CASTERS: usize = 5;
pub(crate) const VERTS: usize = 6;
pub(crate) const TRIS: usize = 7;
pub(crate) const STARTS: usize = 8;
pub(crate) const LISTED: usize = 9;
pub(crate) const MATS: usize = 10;
pub(crate) const COLOR: usize = 11;
pub(crate) const NEAREST: usize = 12;
pub(crate) const SURFACE: usize = 13;
pub(crate) const PICS: usize = 14;
pub(crate) const SHOTS: usize = 15;
pub(crate) const DEPTHS: usize = 16;
pub(crate) const ENV: usize = 17;
pub(crate) const ENV_LEVELS: usize = 18;
pub(crate) const LEFT: usize = 19;
pub(crate) const PICTURES: usize = 20;
pub(crate) const SHOT_PX: usize = 21;
pub(crate) const FACES: usize = 22;
pub(crate) const BOUNCE: usize = 23;
pub(crate) const GLASS: usize = 24;
const SLOTS: usize = 25;

/// A thread's kept buffers (counted in `KEPT_BYTES` while they are kept).
struct Kept([Option<Buffer>; SLOTS]);

impl Kept {
    fn put(&mut self, slot: usize, b: Option<Buffer>) {
        let add = b.as_ref().map_or(0, Buffer::len);
        let gone = std::mem::replace(&mut self.0[slot], b).map_or(0, |o| o.len());
        KEPT_BYTES.fetch_add(add, Ordering::Relaxed);
        KEPT_BYTES.fetch_sub(gone, Ordering::Relaxed);
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        for slot in 0..SLOTS {
            self.put(slot, None);
        }
    }
}

thread_local! {
    static KEPT: RefCell<Kept> = const { RefCell::new(Kept([const { None }; SLOTS])) };
}

/// How many bytes all the drawing threads keep.
static KEPT_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A buffer of at least `len` bytes for what `slot` holds (what was in it is left: fill what is
/// read). Give it back with `keep`.
pub(crate) fn take(slot: usize, len: usize) -> Result<Buffer, String> {
    // the one kept, if it is big enough and not far bigger than needed (else it goes)
    let kept = KEPT.with(|k| {
        let mut k = k.borrow_mut();
        let b = k.0[slot].take();
        KEPT_BYTES.fetch_sub(b.as_ref().map_or(0, Buffer::len), Ordering::Relaxed);
        b
    });
    if let Some(b) = kept
        && b.len() >= len
        && b.len() <= len.max(1 << 20) * 4
    {
        return Ok(b);
    }
    // a little to spare, so a frame a little bigger than the last does not make it again
    Buffer::new(len + len / 4)
}

/// `items` in the buffer for `slot`.
pub(crate) fn take_with<T: Plain>(slot: usize, items: &[T]) -> Result<Buffer, String> {
    let mut b = take(slot, std::mem::size_of_val(items))?;
    b.slice_mut::<T>()[..items.len()].copy_from_slice(items);
    Ok(b)
}

/// The buffer back, for the next frame drawn on this thread (unless the threads keep too much
/// already: then it goes).
pub(crate) fn keep(slot: usize, b: Buffer) {
    if KEPT_BYTES.load(Ordering::Relaxed) + b.len() <= MOST_KEPT {
        KEPT.with(|k| k.borrow_mut().put(slot, Some(b)));
    }
}

/// The most all the drawing threads keep between frames (bytes): a big picture's buffer is
/// kept by a thread or two (a new one costs as much again as filling it), not by all of them.
const MOST_KEPT: usize = 1536 << 20;

/// Lets go of the buffers kept on this thread (a thread that stops drawing for a while).
pub fn let_go() {
    KEPT.with(|k| {
        let mut k = k.borrow_mut();
        for slot in 0..SLOTS {
            k.put(slot, None);
        }
    });
}

/// Says once (on standard error) that the GPU could not draw, and why; the processor draws
/// from then on (a GPU that failed once, or did not answer, is not waited for again).
pub(crate) fn failed(why: &str) {
    static SAID: std::sync::Once = std::sync::Once::new();
    GIVEN_UP.store(true, Ordering::Relaxed);
    SAID.call_once(|| eprintln!("the GPU could not draw 3D, so the processor does: {why}"));
}

/// Whether the GPU has been given up on (for as long as the program runs).
pub(crate) fn given_up() -> bool {
    GIVEN_UP.load(Ordering::Relaxed)
}

static GIVEN_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_structs_are_as_big_as_the_kernel_reads_them() {
        assert_eq!(std::mem::size_of::<Plane>(), 21 * 16);
        assert_eq!(std::mem::size_of::<Caster>(), 14 * 16);
        assert_eq!(std::mem::size_of::<crate::meshes::Face>(), 10 * 16);
        assert_eq!(std::mem::size_of::<Probe>(), 2 * 16);
        assert_eq!(std::mem::size_of::<BounceParams>(), 2 * 16);
        assert_eq!(std::mem::size_of::<Light>(), 6 * 16);
        assert_eq!(std::mem::size_of::<Params>(), 5 * 16);
        assert_eq!(std::mem::size_of::<Vert>(), 7 * 16);
        assert_eq!(std::mem::size_of::<Tri>(), 2 * 16);
        assert_eq!(std::mem::size_of::<Mat>(), 5 * 16);
        assert_eq!(std::mem::size_of::<MeshParams>(), 8 * 16);
    }

    #[test]
    fn the_kernels_build() {
        if !veymelo_os::gpu::available() {
            return;
        }
        veymelo_os::gpu::prepare(&SCENE).unwrap();
        veymelo_os::gpu::prepare(&MESH).unwrap();
        veymelo_os::gpu::prepare(&GATHER).unwrap();
        if veymelo_os::gpu::traces() {
            veymelo_os::gpu::prepare(&SCENE_RAYS).unwrap();
            veymelo_os::gpu::prepare(&MESH_RAYS).unwrap();
            veymelo_os::gpu::prepare(&GATHER_RAYS).unwrap();
        }
    }
}
