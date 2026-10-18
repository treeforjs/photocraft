//! Exports re-open in a third-party reader with the same pixels (FILE-216-4).
//!
//! The other round-trip tests read our exports back with our own importers, so a bug shared by
//! the writer and the reader would pass. Here every flat export and a layered PSD are opened by
//! Pillow (`tests/oracle/pillow_read.py`, run as an external `python3`, never linked) and its
//! pixels are compared with ours.
//!
//! Without `python3` + Pillow the test skips, unless `PHOTOCRAFT_ORACLE_REQUIRED` is non-empty (the
//! Linux CI job sets it and installs python3-pil) so a missing oracle can't pass silently.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::{Rect, Size};
use photocraft_io::*;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/oracle/pillow_read.py");

/// What Pillow decoded: size, mode ("RGBA" or "CMYK") and 8-bit samples.
#[derive(Debug)]
struct Decoded {
    width: u32,
    height: u32,
    mode: String,
    samples: Vec<u8>,
}

/// Runs the oracle on `files`. `None` when python3 or Pillow is missing.
fn pillow(files: &[PathBuf]) -> Option<HashMap<PathBuf, Result<Decoded, String>>> {
    let out = Command::new("python3").arg("-I").arg(SCRIPT).args(files).output().ok()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("No module named 'PIL'"), "oracle script failed: {err}");
        return None;
    }
    let text = String::from_utf8(out.stdout).expect("utf-8 oracle output");
    let mut map = HashMap::new();
    for line in text.lines() {
        let (path, rest) = line.split_once('\t').expect("path<TAB>result");
        let result = match rest.strip_prefix("ERROR ") {
            Some(msg) => Err(msg.to_string()),
            None => {
                let mut it = rest.split(' ');
                let width = it.next().and_then(|v| v.parse().ok()).expect("width");
                let height = it.next().and_then(|v| v.parse().ok()).expect("height");
                let mode = it.next().expect("mode").to_string();
                let samples = it.map(|v| v.parse().expect("sample")).collect();
                Ok(Decoded { width, height, mode, samples })
            }
        };
        map.insert(PathBuf::from(path), result);
    }
    Some(map)
}

fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn single(mode: ColorMode, alpha: bool, size: Size) -> Document {
    let mut d = Document::new("oracle", size, mode, SampleType::U8);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 3, alpha));
    d
}

/// A smooth opaque gradient, so lossy codecs stay close to the source.
fn smooth(size: Size) -> Document {
    let mut d = single(ColorMode::Rgb, false, size);
    let r = d.bounds();
    let fmt = d.pixel_format();
    let n = fmt.channels();
    let (w, h) = (size.width as f32, size.height as f32);
    let mut vals = Vec::new();
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let (fx, fy) = (x as f32 / w, y as f32 / h);
            let px = [0.2 + 0.6 * fx, 0.3 + 0.5 * fy, 0.8 - 0.5 * fx, 1.0];
            vals.extend_from_slice(&px[..n]);
        }
    }
    let mut s = photocraft_raster::Surface::new(fmt);
    s.write_region(r, &vals);
    d.layers[0] = photocraft_doc::Layer::new("Background", photocraft_doc::LayerContent::Raster(s));
    d
}

/// Straight RGBA, 8-bit, of `doc`'s composite.
fn rgba8(doc: &Document) -> Vec<u8> {
    photocraft_compose::flatten(doc).px.iter().flat_map(|p| p.map(to8)).collect()
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

struct Case {
    file: &'static str,
    doc: Document,
    /// Largest per-sample difference allowed between Pillow and the reference.
    tol: u8,
    /// Compare against our own decode of the file (lossy and palette formats) rather than the
    /// source document.
    against_own_decode: bool,
}

fn compare(case: &Case, path: &Path, bytes: &[u8], got: &Decoded) {
    let name = case.file;
    let doc = &case.doc;
    assert_eq!((got.width, got.height), (doc.size.width, doc.size.height), "{name}: size");
    let reference = if case.against_own_decode { import(name, bytes).unwrap_or_else(|e| panic!("{name}: our import: {e}")).document } else { doc.clone() };
    let expected = if got.mode == "CMYK" {
        assert_eq!(doc.mode, ColorMode::Cmyk, "{name}: Pillow decoded CMYK");
        let s = reference.layers[0].surface().expect("raster layer");
        let vals = s.read_region(reference.bounds());
        let n = s.channels();
        vals.chunks_exact(n).flat_map(|px| px[..4].iter().map(|v| to8(*v)).collect::<Vec<_>>()).collect()
    } else {
        assert_eq!(got.mode, "RGBA", "{name}: mode");
        rgba8(&reference)
    };
    assert_eq!(got.samples.len(), expected.len(), "{name}: sample count");
    let stride = 4;
    let mut worst = (0u8, 0usize);
    for (i, (a, b)) in got.samples.chunks_exact(stride).zip(expected.chunks_exact(stride)).enumerate() {
        // Colour under zero alpha is undefined in straight-alpha files.
        let invisible = got.mode == "RGBA" && a[3] == 0 && b[3] == 0;
        for (x, y) in a.iter().zip(b) {
            let d = x.abs_diff(*y);
            if !invisible && d > worst.0 {
                worst = (d, i);
            }
        }
    }
    assert!(worst.0 <= case.tol, "{name} ({}): Pillow differs by {} at pixel {} (tolerance {})", path.display(), worst.0, worst.1, case.tol);
}

fn layered_psd() -> Document {
    let size = Size::new(20, 14);
    let mut d = Document::new("layers", size, ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 1, false));
    let mut top = raster("Top", fmt, Rect::new(3, 2, 15, 11), 2, true);
    top.opacity = g(160);
    d.layers.push(top);
    let mut screen = raster("Screen", fmt, Rect::new(8, 5, 20, 14), 5, false);
    screen.blend = BlendMode::Screen;
    d.layers.push(screen);
    d
}

#[test]
fn exports_reopen_in_pillow_with_the_same_pixels() {
    let size = Size::new(16, 12);
    let cases = vec![
        Case { file: "rgba.png", doc: single(ColorMode::Rgb, true, size), tol: 0, against_own_decode: false },
        Case { file: "gray.png", doc: single(ColorMode::Grayscale, false, size), tol: 0, against_own_decode: false },
        Case { file: "rgba.tiff", doc: single(ColorMode::Rgb, true, size), tol: 0, against_own_decode: false },
        Case { file: "cmyk.tiff", doc: single(ColorMode::Cmyk, false, size), tol: 0, against_own_decode: false },
        Case { file: "rgb.bmp", doc: single(ColorMode::Rgb, false, size), tol: 0, against_own_decode: false },
        Case { file: "rgba.tga", doc: single(ColorMode::Rgb, true, size), tol: 0, against_own_decode: false },
        Case { file: "rgb.ppm", doc: single(ColorMode::Rgb, false, size), tol: 0, against_own_decode: false },
        Case { file: "rgba.qoi", doc: single(ColorMode::Rgb, true, size), tol: 0, against_own_decode: false },
        Case { file: "rgba.ico", doc: single(ColorMode::Rgb, true, size), tol: 0, against_own_decode: false },
        Case { file: "rgba.webp", doc: single(ColorMode::Rgb, true, size), tol: 0, against_own_decode: true },
        Case { file: "rgb.gif", doc: single(ColorMode::Rgb, false, size), tol: 0, against_own_decode: true },
        // Different IDCTs and chroma upsampling may differ by a few levels.
        Case { file: "smooth.jpg", doc: smooth(Size::new(48, 32)), tol: 4, against_own_decode: true },
        // Pillow reads the merged composite; ours comes from the layers.
        Case { file: "layers.psd", doc: layered_psd(), tol: 1, against_own_decode: false },
    ];
    let dir = out_dir();
    let mut written = Vec::new();
    for case in &cases {
        let r = export(&case.doc, case.file, &ExportOptions::default()).unwrap_or_else(|e| panic!("{}: export: {e}", case.file));
        let path = dir.join(case.file);
        std::fs::write(&path, &r.bytes).expect("write export");
        written.push((path, r.bytes));
    }
    let paths: Vec<PathBuf> = written.iter().map(|(p, _)| p.clone()).collect();
    let Some(decoded) = pillow(&paths) else {
        assert!(
            std::env::var("PHOTOCRAFT_ORACLE_REQUIRED").map_or(true, |v| v.is_empty()),
            "python3 with Pillow is required (PHOTOCRAFT_ORACLE_REQUIRED is set)"
        );
        eprintln!("skipping: python3 with Pillow not found");
        return;
    };
    for (case, (path, bytes)) in cases.iter().zip(&written) {
        match decoded.get(path) {
            Some(Ok(got)) => compare(case, path, bytes, got),
            Some(Err(e)) => panic!("{}: Pillow could not open our export: {e}", case.file),
            None => panic!("{}: no oracle result", case.file),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
