//! Golden shaping tests for complex scripts: Arabic joining and ligatures, Hebrew points, Thai
//! marks, Indic reordering and conjuncts, Khmer subscripts, and fallback into each script.
//!
//! They assert what correct OpenType shaping must produce in any decent font for the script
//! (contextual forms differ, marks take no advance, a pre-base matra is drawn left of its
//! consonant), not glyph ids, so they hold across font versions. They need a font for the script:
//! without one a test skips, unless `PHOTOCRAFT_SCRIPT_FONTS_REQUIRED` is set (CI installs
//! `fonts-noto-core` and sets it, so a skip there is a failure).

use photocraft_doc::TextLayer;

use crate::{TextEngine, TextLayout, fonts};

fn layer(text: &str, family: &str) -> TextLayer {
    TextLayer { text: text.into(), font_family: family.into(), size_pt: 40.0, ..Default::default() }
}

/// An engine with system fonts and the first installed family of `families`.
fn engine_with(script: &str, families: &[&str]) -> Option<(TextEngine, String)> {
    let mut e = TextEngine::with_system_fonts();
    match families.iter().find(|f| e.fonts.has_family(f)) {
        Some(f) => Some((e, (*f).to_string())),
        None if std::env::var_os("PHOTOCRAFT_SCRIPT_FONTS_REQUIRED").is_some() => {
            panic!("no {script} font installed (tried {families:?}) but PHOTOCRAFT_SCRIPT_FONTS_REQUIRED is set")
        }
        None => {
            eprintln!("skipped: no {script} font installed (tried {families:?})");
            None
        }
    }
}

fn ids(l: &TextLayout) -> Vec<u32> {
    l.glyphs.iter().map(|g| g.id).collect()
}

fn advance(l: &TextLayout) -> f32 {
    l.lines.iter().map(|l| l.x1 - l.x0).fold(0.0, f32::max)
}

fn no_notdef(l: &TextLayout, what: &str) {
    assert!(!l.glyphs.is_empty(), "{what}: nothing shaped");
    assert!(l.glyphs.iter().all(|g| g.id != 0), "{what}: .notdef in {:?}", ids(l));
}

#[test]
fn arabic_letters_join_and_lam_alef_ligates() {
    let Some((mut e, f)) = engine_with("Arabic", &["Noto Sans Arabic", "Noto Naskh Arabic", "Geeza Pro", "Segoe UI", "Arial"]) else { return };
    let iso = ids(&e.layout(&layer("ب", &f), 72.0));
    let l = e.layout(&layer("ببب", &f), 72.0);
    no_notdef(&l, "beh beh beh");
    let forms = ids(&l);
    assert_eq!(forms.len(), 3, "{forms:?}");
    // Initial, medial and final beh are three different glyphs, none the isolated form.
    assert!(forms.iter().all(|g| !iso.contains(g)), "joined beh drew the isolated form: {forms:?} vs {iso:?}");
    assert!(forms[0] != forms[1] && forms[1] != forms[2] && forms[0] != forms[2], "{forms:?}");
    // Right to left: the logically first letter is drawn rightmost.
    let c: Vec<_> = l.clusters.iter().filter(|c| c.rtl).collect();
    let first = c.iter().min_by_key(|c| c.range.start).map(|c| c.x).unwrap_or_default();
    assert!(c.iter().all(|c| c.x <= first + 1e-3), "{c:?}");
    // Lam + alef is a mandatory ligature: one glyph.
    let la = e.layout(&layer("لا", &f), 72.0);
    no_notdef(&la, "lam alef");
    assert_eq!(la.glyphs.len(), 1, "lam alef did not ligate: {:?}", ids(&la));
}

#[test]
fn hebrew_points_take_no_advance() {
    let Some((mut e, f)) = engine_with("Hebrew", &["Noto Sans Hebrew", "Arial Hebrew", "Segoe UI", "Arial", "DejaVu Sans"]) else { return };
    let plain = e.layout(&layer("שלום", &f), 72.0);
    let pointed = e.layout(&layer("שָׁלוֹם", &f), 72.0);
    no_notdef(&pointed, "pointed shalom");
    assert!(pointed.glyphs.len() > plain.glyphs.len(), "the points are drawn");
    assert!((advance(&pointed) - advance(&plain)).abs() < 0.5, "points moved the letters: {} vs {}", advance(&pointed), advance(&plain));
}

#[test]
fn thai_marks_stack_on_their_base() {
    let Some((mut e, f)) = engine_with("Thai", fonts::THAI_FAMILIES) else { return };
    let base = e.layout(&layer("ผ", &f), 72.0);
    // pho phueng + sara uu (below) + mai tho (above).
    let l = e.layout(&layer("ผู้", &f), 72.0);
    no_notdef(&l, "pho sara uu mai tho");
    assert_eq!(l.glyphs.len(), 3, "{:?}", ids(&l));
    assert!((advance(&l) - advance(&base)).abs() < 0.5, "the marks took advance: {} vs {}", advance(&l), advance(&base));
}

/// Pre-base matra: in "कि" (ka + vowel sign i) the matra is drawn before (left of) the consonant.
fn pre_base_matra(script: &str, families: &[&str], consonant: &str, with_matra: &str) {
    let Some((mut e, f)) = engine_with(script, families) else { return };
    let k = ids(&e.layout(&layer(consonant, &f), 72.0));
    let l = e.layout(&layer(with_matra, &f), 72.0);
    no_notdef(&l, with_matra);
    assert_eq!(l.glyphs.len(), 2, "{:?}", ids(&l));
    let base = l.glyphs.iter().find(|g| k.contains(&g.id)).unwrap_or_else(|| panic!("{script}: consonant glyph {k:?} not in {:?}", ids(&l)));
    let matra = l.glyphs.iter().find(|g| !k.contains(&g.id)).unwrap_or_else(|| panic!("{script}: no matra glyph"));
    assert!(matra.x < base.x, "{script}: the matra (x {}) is not left of the consonant (x {})", matra.x, base.x);
}

#[test]
fn devanagari_i_matra_reorders_and_ksha_forms_a_conjunct() {
    let fam = &["Noto Sans Devanagari", "Kohinoor Devanagari", "Devanagari Sangam MN", "Nirmala UI", "Mangal"];
    pre_base_matra("Devanagari", fam, "क", "कि");
    let Some((mut e, f)) = engine_with("Devanagari", fam) else { return };
    // ka + virama + ssa is the conjunct ksha: no visible virama, fewer glyphs than characters.
    let l = e.layout(&layer("क्ष", &f), 72.0);
    no_notdef(&l, "ksha");
    assert!(l.glyphs.len() < 3, "ksha shaped as separate letters: {:?}", ids(&l));
    let l = e.layout(&layer("नमस्ते", &f), 72.0);
    no_notdef(&l, "namaste");
}

#[test]
fn bengali_i_matra_reorders() {
    pre_base_matra("Bengali", &["Noto Sans Bengali", "Kohinoor Bangla", "Bangla Sangam MN", "Nirmala UI", "Vrinda"], "ক", "কি");
}

#[test]
fn tamil_two_part_vowel_surrounds_its_consonant() {
    let Some((mut e, f)) = engine_with("Tamil", &["Noto Sans Tamil", "Tamil Sangam MN", "Nirmala UI", "Latha"]) else { return };
    let k = ids(&e.layout(&layer("க", &f), 72.0));
    // ka + vowel sign o splits into e (before) and aa (after).
    let l = e.layout(&layer("கொ", &f), 72.0);
    no_notdef(&l, "ko");
    assert_eq!(l.glyphs.len(), 3, "{:?}", ids(&l));
    let mut by_x: Vec<_> = l.glyphs.iter().collect();
    by_x.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert!(by_x.get(1).is_some_and(|g| k.contains(&g.id)), "ka is not between the vowel parts: {by_x:?} (ka {k:?})");
}

#[test]
fn khmer_coeng_makes_a_subscript() {
    let Some((mut e, f)) = engine_with("Khmer", &["Noto Sans Khmer", "Khmer Sangam MN", "Khmer MN", "Khmer UI", "Leelawadee UI"]) else { return };
    // ka + coeng + ka: the second ka is a subscript under the first, the coeng isn't drawn.
    let l = e.layout(&layer("ក្ក", &f), 72.0);
    no_notdef(&l, "ka coeng ka");
    assert_eq!(l.glyphs.len(), 2, "{:?}", ids(&l));
    let single = e.layout(&layer("ក", &f), 72.0);
    assert!(advance(&l) < advance(&single) * 1.5, "the subscript was set beside its base");
}

#[test]
fn complex_scripts_in_a_latin_font_fall_back_to_an_installed_font() {
    let mut e = TextEngine::with_system_fonts();
    let samples = [
        ("Arabic", "السلام عليكم"),
        ("Hebrew", "שלום עולם"),
        ("Devanagari", "नमस्ते दुनिया"),
        ("Bengali", "নমস্কার"),
        ("Tamil", "வணக்கம்"),
        ("Khmer", "សួស្តី"),
        ("Thai", "สวัสดี"),
    ];
    let required = std::env::var_os("PHOTOCRAFT_SCRIPT_FONTS_REQUIRED").is_some();
    for (script, text) in samples {
        let l = e.layout(&layer(text, "Inter"), 72.0);
        let drawn = !l.glyphs.is_empty() && l.glyphs.iter().all(|g| g.id != 0);
        if required {
            assert!(drawn, "{script} in Inter drew .notdef although script fonts are installed: {:?}", ids(&l));
        } else if !drawn {
            eprintln!("skipped {script}: no installed fallback font covers it");
        }
    }
}

#[test]
fn complex_script_fallback_candidates_cover_every_platform() {
    // Runs without the fonts: each platform has a Khmer and an Indic family in the fallback
    // candidates, ahead of the broad last-resort fonts.
    for order in [crate::cjk::script_order(None), crate::cjk::script_order(Some("ja"))] {
        let fb = fonts::fallback_candidates(&order);
        let last = fb.iter().position(|f| *f == "Arial Unicode MS").unwrap();
        for fam in ["Nirmala UI", "Khmer UI", "Kohinoor Devanagari", "Khmer Sangam MN", "Tamil Sangam MN", "Noto Sans Devanagari", "Noto Sans Khmer"] {
            let i = fb.iter().position(|f| *f == fam).unwrap_or_else(|| panic!("{fam} missing from {fb:?}"));
            assert!(i < last, "{fam} after the last-resort fonts");
        }
    }
}
