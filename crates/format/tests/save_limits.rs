//! Saving must never replace a readable bundle with one the normal loader rejects.
mod common;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, LayerId, Size};
use photocraft_format::{FormatError, LoadOptions, PcraftWriter, SaveOptions, load_from_bytes, load_from_bytes_with, load_path};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

fn document() -> Document {
    let mut doc = Document::with_background("budget", Size::new(4, 4), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers.first_mut().unwrap().surface_mut().unwrap().write_pixel(0, 0, &[0.2, 0.4, 0.6, 1.0]);
    doc.icc_profile = Some(Arc::new((0..17).collect()));
    doc
}
fn files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                out.insert(path.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(path).unwrap());
            }
        }
    }
    out
}

#[test]
fn exact_save_budgets_roundtrip_and_shared_tiles_count_as_separate_load_allocations() {
    let mut doc = document();
    // Shared content has one stored object but loading allocates each tile reference.
    let mut duplicate = doc.layers.first().unwrap().clone();
    duplicate.id = LayerId::fresh();
    doc.layers.push(duplicate);
    // Shared blobs, in contrast, are cached by hash on load and count once.
    doc.metadata.exif = doc.icc_profile.clone();
    let tile_bytes = doc.layers.first().unwrap().surface().unwrap().tiles().map(|(_, tile)| tile.bytes().len() as u64).sum::<u64>();
    let (_, stats) = PcraftWriter::new().save_zip(&doc, &SaveOptions::default()).unwrap();
    let limits = LoadOptions { max_manifest_bytes: stats.manifest_bytes, max_blob_bytes: 17, max_total_bytes: tile_bytes * 2 + 17, ..Default::default() };
    let mut writer = PcraftWriter::new();
    writer.set_save_limits(limits);
    let (bytes, _) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes_with(&bytes, &limits).unwrap(), doc);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    writer.set_save_limits(LoadOptions { max_total_bytes: limits.max_total_bytes - 1, ..limits });
    assert!(matches!(writer.save_zip(&doc, &SaveOptions::default()), Err(FormatError::LimitExceeded(_))));
}

#[test]
fn refused_manifest_blob_or_expansion_preserves_existing_zip_and_every_directory_file() {
    let doc = document();
    let root = common::temp_dir("save-budgets");
    let zip = root.join("saved.pcraft");
    let dir = root.join("directory");
    let mut baseline = PcraftWriter::new();
    baseline.save_path(&doc, &zip, &SaveOptions::default()).unwrap();
    baseline.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    let original_zip = std::fs::read(&zip).unwrap();
    let original_files = files(&dir);
    let mut changed = doc.clone();
    changed.name = "different document name".into();
    for limits in [
        LoadOptions { max_manifest_bytes: 1, ..Default::default() },
        LoadOptions { max_blob_bytes: 16, ..Default::default() },
        LoadOptions { max_total_bytes: 1, ..Default::default() },
    ] {
        let mut writer = PcraftWriter::new();
        writer.set_save_limits(limits);
        assert!(matches!(writer.save_path(&changed, &zip, &SaveOptions::default()), Err(FormatError::LimitExceeded(_))));
        assert_eq!(std::fs::read(&zip).unwrap(), original_zip);
        assert!(matches!(writer.save_dir(&changed, &dir, &SaveOptions::default()), Err(FormatError::LimitExceeded(_))));
        assert_eq!(files(&dir), original_files);
        let missing = root.join("must-not-be-created");
        assert!(writer.save_dir(&changed, &missing, &SaveOptions::default()).is_err());
        assert!(!missing.exists());
        assert_eq!(load_path(&zip).unwrap(), doc);
        assert_eq!(load_path(&dir).unwrap(), doc);
    }
    std::fs::remove_dir_all(root).unwrap();
}
