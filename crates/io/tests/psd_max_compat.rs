//! Maximize PSD and PSB File Compatibility (Preferences › File Handling): off, a layered file
//! carries a white placeholder instead of the merged composite, flagged in resource 1057, and
//! still opens with its layers.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_geom::Rect;
use photocraft_io::*;
use photocraft_psd::PsdFile;

fn red_square(mode: ColorMode, depth: SampleType) -> Document {
    let mut d = Document::new("m", Size::new(8, 6), mode, depth);
    let fmt = d.pixel_format();
    let mut a = Layer::raster("a", fmt);
    a.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 8, 6), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
    d.layers = vec![a];
    d
}

fn save(d: &Document, maximize: bool) -> Vec<u8> {
    export(d, "x.psd", &ExportOptions { maximize_compatibility: maximize, ..Default::default() }).unwrap().bytes
}

#[test]
fn off_writes_a_white_placeholder_and_keeps_the_layers() {
    let d = red_square(ColorMode::Rgb, SampleType::U8);
    let on = PsdFile::from_bytes(&save(&d, true)).unwrap();
    assert_eq!(on.has_real_merged_data(), Some(true));
    assert_eq!(on.composite_rgba8().unwrap().data.get(0..4), Some(&[255, 0, 0, 255][..]));

    let bytes = save(&d, false);
    let off = PsdFile::from_bytes(&bytes).unwrap();
    assert_eq!(off.has_real_merged_data(), Some(false));
    assert!(off.composite_rgba8().unwrap().data.chunks(4).all(|p| p == [255, 255, 255, 255]), "white placeholder");
    let back = import("x.psd", &bytes).unwrap().document;
    assert_eq!(back.layers.len(), 1);
    let px = photocraft_compose::flatten(&back);
    assert!(px.px.iter().all(|p| (p[0] - 1.0).abs() < 1e-3 && p[1] < 1e-3), "the layers still carry the image");
}

#[test]
fn placeholder_is_white_in_every_mode() {
    for (mode, depth) in
        [(ColorMode::Cmyk, SampleType::U8), (ColorMode::Lab, SampleType::U16), (ColorMode::Grayscale, SampleType::U16), (ColorMode::Rgb, SampleType::F32)]
    {
        let d = red_square(mode, depth);
        let off = PsdFile::from_bytes(&save(&d, false)).unwrap();
        let px = merged_composite(&off).unwrap();
        assert_eq!(px.len(), 48, "{mode:?} {depth:?}");
        assert!(px.iter().all(|p| p.iter().all(|v| (v - 1.0).abs() < 0.02)), "{mode:?} {depth:?}: {:?}", px.first());
    }
}

#[test]
fn a_file_without_layers_always_gets_the_real_composite() {
    let d = Document::new("e", Size::new(4, 3), ColorMode::Rgb, SampleType::U8);
    let off = PsdFile::from_bytes(&save(&d, false)).unwrap();
    if off.layers().is_empty() {
        assert_eq!(off.has_real_merged_data(), Some(true));
    }
}
