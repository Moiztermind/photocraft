//! Image → AI Upscale 2× (Fast / Best). Fork-only (Moiztermind/photocraft, `moiztermind/stable`);
//! never part of an upstream PR.
//!
//! Doubles the document exactly like Image Size with Lanczos (masks, channels, selection, vectors,
//! effects and guides all follow), then replaces each pixel layer's on-canvas pixels with a
//! Real-ESRGAN upscale of its original pixels. Real-ESRGAN runs as the external, portable
//! `realesrgan-ncnn-vulkan` program (BSD-3-Clause, github.com/xinntao/Real-ESRGAN) found by
//! [`find_tools`]; nothing is linked in. It runs as a background job: progress comes from the
//! program's own percentages, and Cancel kills it.
//!
//! - Fast: `realesr-animevideov3` at 2× (about 7 s for a 500 × 1000 px photo on an Intel UHD 620).
//! - Best: `realesrgan-x4plus`, computed at 4× and brought down to 2× by the program (about 150 s).
//!
//! RGB and Grayscale at 8 or 16 bits. The program works in 8 bits, so a 16-bit layer comes back
//! with 8-bit detail. The colour values go through unchanged (no profile conversion).

use photocraft_algo::resample::Resample;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const FAST: &str = "image.aiUpscaleFast";
pub const BEST: &str = "image.aiUpscaleBest";

/// Environment variable naming a folder that holds the program and its `models` folder.
pub const ENV_DIR: &str = "PHOTOCRAFT_REALESRGAN";

#[cfg(windows)]
const EXE: &str = "realesrgan-ncnn-vulkan.exe";
#[cfg(not(windows))]
const EXE: &str = "realesrgan-ncnn-vulkan";

/// One Real-ESRGAN model: its file stem in `models/` and its menu wording.
#[derive(Clone, Copy, Debug)]
struct Model {
    file: &'static str,
    label: &'static str,
}

const FAST_MODEL: Model = Model { file: "realesr-animevideov3-x2", label: "Fast" };
const BEST_MODEL: Model = Model { file: "realesrgan-x4plus", label: "Best" };

impl Model {
    /// The `-n` name the program takes: the file stem without the scale suffix.
    fn name(self) -> &'static str {
        self.file.strip_suffix("-x2").unwrap_or(self.file)
    }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    supported(&d.doc)
}

/// Why `doc` can't be upscaled, if it can't.
fn supported(doc: &Document) -> std::result::Result<(), String> {
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err("AI Upscale works on RGB and Grayscale images: convert the image to RGB first".into());
    }
    if doc.depth == SampleType::F32 {
        return Err("AI Upscale works on 8 and 16 bits per channel: convert the image to 16 bits first".into());
    }
    Ok(())
}

/// Where the program and model files are.
#[derive(Clone, Debug)]
struct Tools {
    exe: std::path::PathBuf,
    models: std::path::PathBuf,
}

/// The folders searched for the program, in order: `$PHOTOCRAFT_REALESRGAN`, `realesrgan` next to
/// the PhotoCraft executable (the stable build's zip), and Coaster Prep's
/// `%USERPROFILE%\Lasers\templates\tools`.
#[cfg(not(target_arch = "wasm32"))]
fn candidate_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os(ENV_DIR).filter(|d| !d.is_empty()) {
        v.push(std::path::PathBuf::from(d));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("realesrgan"))) {
        v.push(dir);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        v.push(std::path::PathBuf::from(home).join("Lasers").join("templates").join("tools"));
    }
    v
}

#[cfg(not(target_arch = "wasm32"))]
fn find_tools(model: Model) -> Result<Tools> {
    for dir in candidate_dirs() {
        let exe = dir.join(EXE);
        let models = dir.join("models");
        let has_model = ["param", "bin"].iter().all(|ext| models.join(format!("{}.{ext}", model.file)).is_file());
        if exe.is_file() && has_model {
            return Ok(Tools { exe, models });
        }
    }
    Err(EngineError::Other(format!(
        "AI Upscale needs Real-ESRGAN ({EXE} and models\\{}.param/.bin): put it in a \"realesrgan\" folder next to PhotoCraft, or set {ENV_DIR} to its folder",
        model.file
    )))
}

#[cfg(target_arch = "wasm32")]
fn find_tools(_model: Model) -> Result<Tools> {
    Err(EngineError::Other("AI Upscale needs the desktop app".into()))
}

fn run(s: &mut Session, best: bool) -> Result<Value> {
    let (cmd, model) = if best { (BEST, BEST_MODEL) } else { (FAST, FAST_MODEL) };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    supported(&d.doc).map_err(EngineError::Other)?;
    let (ow, oh) = (d.doc.size.width, d.doc.size.height);
    let (nw, nh) = (ow.saturating_mul(2), oh.saturating_mul(2));
    if nw > 300_000 || nh > 300_000 {
        return Err(EngineError::Other(format!("{nw} x {nh} px is past the 300000 px side limit")));
    }
    crate::image_cmds::check_resample_budget(cmd, &d.doc, nw, nh, "")?;
    let tools = find_tools(model)?;
    // Color Sampler points move with the pixels, as Image Size moves them.
    s.set_sampler_map(photocraft_geom::Affine { m: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0] });
    crate::jobs::edit_job(
        s,
        &format!("AI Upscale ({})", model.label),
        move |doc, _, ctx| upscale_doc(doc, &tools, model, ctx),
        move |layers| json!({ "width": nw, "height": nh, "layers": layers }),
    )
}

/// The body of the job: Image Size 2× with Lanczos, then each pixel layer's on-canvas area
/// through Real-ESRGAN. Returns how many layers were upscaled.
fn upscale_doc(doc: &mut Document, tools: &Tools, model: Model, ctx: &crate::jobs::JobCtx) -> Result<usize> {
    let canvas = Rect::from_xywh(0, 0, doc.size.width, doc.size.height);
    let mut originals: Vec<(LayerId, Surface)> = Vec::new();
    crate::image_cmds::for_each_layer(&mut doc.layers, &mut |l| {
        if let LayerContent::Raster(surf) = &l.content {
            originals.push((l.id, surf.clone()));
        }
    });
    ctx.progress(0.01, "Resizing");
    crate::image_cmds::resample_doc(doc, canvas.width() * 2, canvas.height() * 2, Resample::Lanczos);
    let format = doc.pixel_format();
    let n = originals.len().max(1) as f32;
    let mut done = 0;
    for (i, (id, original)) in originals.into_iter().enumerate() {
        ctx.check()?;
        let area = original.content_bounds().intersect(&canvas);
        if area.is_empty() {
            continue;
        }
        let (lo, hi) = (0.02 + 0.96 * i as f32 / n, 0.02 + 0.96 * (i + 1) as f32 / n);
        ctx.progress(lo, &format!("Upscaling layer {} of {}", i + 1, n as usize));
        let rgba = upscale_area(&original, area, tools, model, ctx, lo, hi)?;
        let target = Rect::from_xywh(area.x0 * 2, area.y0 * 2, area.width() * 2, area.height() * 2);
        let values = to_format(&rgba, &format, area_opaque(&original, area));
        let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let LayerContent::Raster(surf) = &mut layer.content {
            surf.try_write_region(target, &values)?;
            done += 1;
        }
    }
    crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::All);
    ctx.progress(1.0, "Done");
    Ok(done)
}

/// Is every pixel of `area` fully opaque? Then the program gets RGB and alpha stays 1.
fn area_opaque(s: &Surface, area: Rect) -> bool {
    !s.format().alpha || read_rgba8(s, area).iter().all(|p| p[3] == 255)
}

fn read_rgba8(s: &Surface, area: Rect) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; area.width() as usize * area.height() as usize];
    s.read_rgba8_into(area, &mut px);
    px
}

/// Straight 8-bit RGBA (2× the area) to the document's channel values.
fn to_format(rgba: &[u8], format: &photocraft_color::PixelFormat, opaque: bool) -> Vec<f32> {
    let n = format.channels();
    let mut out = vec![0.0f32; rgba.len() / 4 * n];
    for (px, dst) in rgba.as_chunks::<4>().0.iter().zip(out.chunks_exact_mut(n)) {
        let a = if opaque { 1.0 } else { f32::from(px[3]) / 255.0 };
        let c = [f32::from(px[0]) / 255.0, f32::from(px[1]) / 255.0, f32::from(px[2]) / 255.0, a];
        photocraft_raster::from_rgba_into(format, c, dst);
    }
    out
}

/// Runs the program on `area` of `s` and returns 2× straight RGBA8 (row-major, `4·2w·2h` bytes).
#[cfg(not(target_arch = "wasm32"))]
fn upscale_area(s: &Surface, area: Rect, tools: &Tools, model: Model, ctx: &crate::jobs::JobCtx, lo: f32, hi: f32) -> Result<Vec<u8>> {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, PngCompression};

    let (w, h) = (area.width(), area.height());
    let opaque = area_opaque(s, area);
    let px = read_rgba8(s, area);
    let (layout, bytes): (ChannelLayout, Vec<u8>) = if opaque {
        (ChannelLayout::Rgb, px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect())
    } else {
        (ChannelLayout::Rgba, px.into_iter().flatten().collect())
    };
    let img = Image::from_u8(w, h, layout, bytes).map_err(|e| EngineError::Other(format!("AI Upscale: {e}")))?;
    let opts = EncodeOptions { png_compression: PngCompression::Fast, ..EncodeOptions::default() };
    let png = photocraft_codecs::encode(&img, Format::Png, &opts).map_err(|e| EngineError::Other(format!("AI Upscale: {e}")))?;

    let dir = TempDir::new()?;
    let (input, output) = (dir.path.join("in.png"), dir.path.join("out.png"));
    std::fs::write(&input, png).map_err(|e| EngineError::Other(format!("AI Upscale: could not write {}: {e}", input.display())))?;
    run_program(tools, model, &input, &output, ctx, lo, hi)?;
    let bytes = std::fs::read(&output).map_err(|e| EngineError::Other(format!("AI Upscale: the upscaler wrote no image ({e})")))?;
    let out = photocraft_codecs::decode(&bytes).map_err(|e| EngineError::Other(format!("AI Upscale: could not read the upscaled image: {e}")))?;
    if out.dimensions() != (w * 2, h * 2) {
        return Err(EngineError::Other(format!("AI Upscale: expected {} x {} px back, got {} x {}", w * 2, h * 2, out.width(), out.height())));
    }
    Ok(out.to_rgba8())
}

#[cfg(target_arch = "wasm32")]
fn upscale_area(_: &Surface, _: Rect, _: &Tools, _: Model, _: &crate::jobs::JobCtx, _: f32, _: f32) -> Result<Vec<u8>> {
    Err(EngineError::Other("AI Upscale needs the desktop app".into()))
}

/// Runs the program, mapping its percentages onto `lo..hi` and killing it on Cancel.
#[cfg(not(target_arch = "wasm32"))]
fn run_program(tools: &Tools, model: Model, input: &std::path::Path, output: &std::path::Path, ctx: &crate::jobs::JobCtx, lo: f32, hi: f32) -> Result<()> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(&tools.exe);
    cmd.arg("-i").arg(input).arg("-o").arg(output).args(["-n", model.name(), "-s", "2", "-f", "png"]).arg("-m").arg(&tools.models);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| EngineError::Other(format!("AI Upscale: could not start {}: {e}", tools.exe.display())))?;
    // The program reports "12.50%" lines on stderr; keep the rest for the error message.
    let reader = child.stderr.take().map(|mut err| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (mut log, mut line, mut buf) = (String::new(), String::new(), [0u8; 256]);
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for &b in buf.get(..n).unwrap_or_default() {
                    if b == b'\n' || b == b'\r' {
                        match line.trim().strip_suffix('%').and_then(|v| v.trim().parse::<f32>().ok()) {
                            Some(pct) => ctx.progress(lo + (hi - lo) * (pct / 100.0).clamp(0.0, 1.0), ""),
                            None if !line.trim().is_empty() => {
                                log.push_str(line.trim());
                                log.push('\n');
                            }
                            None => {}
                        }
                        line.clear();
                    } else if line.len() < 4096 {
                        line.push(char::from(b));
                    }
                }
            }
            log
        })
    });
    let status = loop {
        if ctx.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(EngineError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(EngineError::Other(format!("AI Upscale: {e}"))),
        }
    };
    let log = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    if !status.success() || !output.is_file() {
        let why = log.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no message");
        return Err(EngineError::Other(format!("AI Upscale: the upscaler failed ({status}): {why}")));
    }
    Ok(())
}

/// A scratch folder removed when dropped.
#[cfg(not(target_arch = "wasm32"))]
struct TempDir {
    path: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl TempDir {
    fn new() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("photocraft-upscale-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|e| EngineError::Other(format!("AI Upscale: could not create {}: {e}", path.display())))?;
        Ok(Self { path })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FAST,
            label: "AI Upscale 2× (Fast)",
            menu: &["Image"],
            shortcut: None,
            params: r##"{} → {width,height,layers} (doubles the document; pixel layers through Real-ESRGAN realesr-animevideov3, about 7 s per 0.5 MP; needs realesrgan-ncnn-vulkan, see PHOTOCRAFT_REALESRGAN)"##,
            enabled,
            run: |s, _| run(s, false),
            journal: true,
        },
        CommandSpec {
            id: BEST,
            label: "AI Upscale 2× (Best)",
            menu: &["Image"],
            shortcut: None,
            params: r##"{} → {width,height,layers} (as Fast with realesrgan-x4plus: sharper on photos, about 20× slower)"##,
            enabled,
            run: |s, _| run(s, true),
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 6, "mode": mode, "depth": depth})).unwrap();
        s
    }

    #[test]
    fn model_names_match_the_program() {
        assert_eq!(FAST_MODEL.name(), "realesr-animevideov3");
        assert_eq!(BEST_MODEL.name(), "realesrgan-x4plus");
    }

    #[test]
    fn refuses_cmyk_lab_and_32_bit_without_changing_the_document() {
        for (mode, depth) in [("cmyk", 8), ("lab", 8), ("rgb", 32)] {
            let mut s = session(mode, depth);
            assert!(s.execute(FAST, json!({})).is_err(), "{mode} {depth}");
            assert_eq!(s.active().unwrap().doc.size.width, 8);
        }
    }

    #[test]
    fn no_document_is_an_error() {
        let mut s = Session::new();
        assert!(s.execute(FAST, json!({})).is_err());
        assert!(s.execute(BEST, json!({})).is_err());
    }

    #[test]
    fn missing_program_is_an_error_naming_the_folder() {
        // A folder that has no program: the env folder is searched first, then the others, so
        // only assert the message when nothing else on this machine provides it.
        let tools = find_tools(Model { file: "no-such-model-x2", label: "Test" });
        let err = tools.unwrap_err().to_string();
        assert!(err.contains("realesrgan") && err.contains(ENV_DIR), "{err}");
    }

    #[test]
    fn to_format_writes_gray_and_rgb_with_alpha() {
        let rgba = [255u8, 0, 0, 128, 0, 255, 0, 255];
        let rgb = to_format(&rgba, &photocraft_color::PixelFormat::RGBA8, false);
        assert_eq!(rgb.len(), 8);
        assert!((rgb[3] - 128.0 / 255.0).abs() < 1e-6);
        let opaque = to_format(&rgba, &photocraft_color::PixelFormat::RGBA8, true);
        assert_eq!(opaque[3], 1.0);
    }

    /// Runs the real program when it is installed (this laptop has Coaster Prep's copy):
    /// `cargo test -p photocraft-engine --lib ai_upscale -- --ignored`.
    #[test]
    #[ignore = "needs realesrgan-ncnn-vulkan and a Vulkan GPU"]
    fn upscales_an_rgb_document_with_the_real_program() {
        let mut s = session("rgb", 8);
        let r = s.execute(FAST, json!({})).unwrap();
        assert_eq!(r["width"], 16);
        assert_eq!(r["height"], 12);
        assert_eq!(r["layers"], 1);
        assert_eq!(s.active().unwrap().doc.size.width, 16);
    }
}
