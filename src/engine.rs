//! Chooses who rasterises each frame: the CPU, the GPU, or both at once.
//!
//! * `Cpu`: the software rasteriser draws everything.
//! * `Gpu`: a `wgpu` pipeline draws everything and the result is read back.
//! * `Both`: the frame is split in two. The GPU draws the top half on its own
//!   thread while the CPU draws the bottom half, then the halves are joined.
//!
//! If the GPU is missing, not compiled in, or fails mid-run, the engine falls
//! back to the CPU (once, with a reason) instead of showing nothing.

use crate::gpu::{GpuImage, GpuInfo, GpuParams, GpuRenderer};
use crate::image::Image;
use crate::mesh::Mesh;
use crate::raster::{render_textured, Frame, RenderOpts};
use crate::settings::Backend;

/// What actually drew a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub backend: Backend,
    /// Set when the requested backend could not be used.
    pub fallback_reason: Option<String>,
}

#[derive(Default)]
pub struct Engine {
    gpu: Option<GpuRenderer>,
    /// Why the GPU can't be used (sticky, so we do not retry every frame).
    gpu_error: Option<String>,
}

impl Engine {
    pub fn new() -> Engine {
        Engine::default()
    }

    /// Opens the GPU on first use. `Err` carries a human-readable reason.
    fn gpu(&mut self) -> Result<&mut GpuRenderer, String> {
        if let Some(e) = &self.gpu_error {
            return Err(e.clone());
        }
        if self.gpu.is_none() {
            match GpuRenderer::new() {
                Ok(g) => self.gpu = Some(g),
                Err(e) => {
                    self.gpu_error = Some(e.clone());
                    return Err(e);
                }
            }
        }
        Ok(self.gpu.as_mut().expect("just created"))
    }

    /// The adapter in use, if the GPU has been opened successfully.
    pub fn gpu_info(&self) -> Option<&GpuInfo> {
        self.gpu.as_ref().map(|g| g.info())
    }

    /// Draws `mesh` into `frame` (which must be cleared) using `backend`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        backend: Backend,
        mesh: &Mesh,
        tex: Option<&Image>,
        (ax, ay, cam): (f32, f32, f32),
        opts: RenderOpts,
        frame: &mut Frame,
    ) -> Outcome {
        let (w, h) = (frame.canvas.w, frame.canvas.h);
        let ppr = opts.pixels_per_row.max(1);
        let cpu_all = |frame: &mut Frame, reason: Option<String>| {
            render_textured(mesh, ax, ay, cam, opts, tex, frame);
            Outcome {
                backend: Backend::Cpu,
                fallback_reason: reason,
            }
        };
        if !backend.uses_gpu() || w == 0 || h == 0 {
            return cpu_all(frame, None);
        }
        let params = |height: usize| GpuParams {
            width: w,
            height,
            full_h: h,
            pixels_per_row: ppr,
            ax,
            ay,
            cam,
            smooth: opts.smooth,
            two_sided: opts.two_sided,
        };
        let gpu = match self.gpu() {
            Ok(g) => g,
            Err(e) => return cpu_all(frame, Some(e)),
        };

        if backend == Backend::Gpu {
            return match gpu.render(mesh, tex, &params(h)) {
                Ok(img) => {
                    paste(frame, &img);
                    Outcome {
                        backend,
                        fallback_reason: None,
                    }
                }
                Err(e) => self.gpu_failed(e, |reason| cpu_all(frame, Some(reason))),
            };
        }

        // Both: GPU takes the top rows (a whole number of terminal rows), CPU the rest.
        let split = (h / ppr / 2) * ppr;
        if split == 0 {
            return cpu_all(frame, None);
        }
        let gpu_params = params(split);
        let cpu_opts = RenderOpts {
            y_range: Some((split, h)),
            ..opts
        };
        let (gpu_result, ()) = std::thread::scope(|s| {
            let job = s.spawn(move || gpu.render(mesh, tex, &gpu_params));
            render_textured(mesh, ax, ay, cam, cpu_opts, tex, frame);
            (
                job.join().unwrap_or_else(|_| Err("GPU thread panicked".into())),
                (),
            )
        });
        match gpu_result {
            Ok(img) => {
                paste(frame, &img);
                Outcome {
                    backend,
                    fallback_reason: None,
                }
            }
            Err(e) => {
                // The CPU already drew the bottom; draw the top half on the CPU too.
                self.gpu_failed(e, |reason| {
                    let top = RenderOpts {
                        y_range: Some((0, split)),
                        ..opts
                    };
                    render_textured(mesh, ax, ay, cam, top, tex, frame);
                    Outcome {
                        backend: Backend::Cpu,
                        fallback_reason: Some(reason),
                    }
                })
            }
        }
    }

    fn gpu_failed(&mut self, err: String, then: impl FnOnce(String) -> Outcome) -> Outcome {
        self.gpu = None;
        self.gpu_error = Some(err.clone());
        then(err)
    }
}

/// Copies a GPU band into the top rows of the frame's canvas.
fn paste(frame: &mut Frame, img: &GpuImage) {
    let c = &mut frame.canvas;
    let n = (img.w * img.h).min(c.lum.len()).min(img.lum.len());
    if img.w == c.w {
        c.lum[..n].copy_from_slice(&img.lum[..n]);
        c.base[..n].copy_from_slice(&img.base[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(f: &Frame) -> usize {
        f.canvas.lum.iter().filter(|&&l| l >= 0.0).count()
    }

    fn draw(engine: &mut Engine, backend: Backend, ppr: usize) -> (Frame, Outcome) {
        let mut f = Frame::new(60, 24 * ppr);
        let opts = RenderOpts {
            pixels_per_row: ppr,
            ..RenderOpts::default()
        };
        let o = engine.render(backend, &Mesh::cube(), None, (0.4, 0.6, 5.0), opts, &mut f);
        (f, o)
    }

    #[test]
    fn cpu_backend_is_a_plain_render() {
        let (f, o) = draw(&mut Engine::new(), Backend::Cpu, 1);
        assert_eq!(o.backend, Backend::Cpu);
        assert!(o.fallback_reason.is_none());
        assert!(covered(&f) > 100);
    }

    #[test]
    fn gpu_requests_never_leave_the_screen_blank() {
        // With or without a usable GPU the frame must contain the model.
        for backend in [Backend::Gpu, Backend::Both] {
            for ppr in [1, 2] {
                let (f, o) = draw(&mut Engine::new(), backend, ppr);
                assert!(covered(&f) > 100, "{backend:?} ppr={ppr}");
                if o.backend == Backend::Cpu {
                    assert!(o.fallback_reason.is_some(), "a CPU fallback must say why");
                }
            }
        }
    }

    #[test]
    fn fallback_is_sticky_and_matches_cpu() {
        let mut e = Engine::new();
        e.gpu_error = Some("test: no gpu".into());
        let (f, o) = draw(&mut e, Backend::Both, 1);
        assert_eq!(o.backend, Backend::Cpu);
        assert_eq!(o.fallback_reason.as_deref(), Some("test: no gpu"));
        let (cpu, _) = draw(&mut Engine::new(), Backend::Cpu, 1);
        assert_eq!(f.canvas.lum, cpu.canvas.lum);
    }

    #[test]
    fn zero_sized_frames_are_fine() {
        let mut f = Frame::new(0, 0);
        let o = Engine::new().render(
            Backend::Both,
            &Mesh::cube(),
            None,
            (0.0, 0.0, 5.0),
            RenderOpts::default(),
            &mut f,
        );
        assert_eq!(o.backend, Backend::Cpu);
    }

    /// With a real adapter, CPU, GPU and CPU+GPU frames agree.
    #[cfg(feature = "gpu")]
    #[test]
    fn all_three_backends_agree_when_a_gpu_exists() {
        let mut e = Engine::new();
        if e.gpu().is_err() {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        for ppr in [1, 2] {
            let (cpu, _) = draw(&mut e, Backend::Cpu, ppr);
            for backend in [Backend::Gpu, Backend::Both] {
                let (f, o) = draw(&mut e, backend, ppr);
                assert_eq!(o.backend, backend, "{:?}", o.fallback_reason);
                let diff = f
                    .canvas
                    .lum
                    .iter()
                    .zip(&cpu.canvas.lum)
                    .filter(|(a, b)| (**a >= 0.0) != (**b >= 0.0) || (**a - **b).abs() > 0.03)
                    .count();
                assert!(
                    diff * 100 <= covered(&cpu),
                    "{backend:?} ppr={ppr}: {diff} pixels differ"
                );
            }
        }
    }
}
