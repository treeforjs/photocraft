//! The EXIF ColorSpace tag (0xA001 in the Exif IFD that IFD0's ExifIFDPointer, tag 0x8769,
//! points to). EXIF 2.3 §4.6.5: 1 = sRGB, 0xFFFF = uncalibrated. Cameras that write no ICC
//! profile record sRGB here, and Photoshop treats such a file as tagged sRGB unless
//! Preferences › File Handling › Ignore EXIF Profile Tag is on.
//!
//! The block is untrusted: offsets and counts are bounds-checked, both byte orders and
//! BigTIFF-style EXIF are handled, and anything malformed reads as "not sRGB".

use crate::orientation::{Order, ifd0_entries, tiff_body};

const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_COLOR_SPACE: u16 = 0xA001;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_IFD: u16 = 13;
const TYPE_LONG8: u16 = 16;
const TYPE_IFD8: u16 = 18;
/// ColorSpace value for sRGB.
const SRGB: u16 = 1;

/// `true` when `exif` (with or without the JPEG `Exif\0\0` prefix) says its colours are sRGB.
pub fn exif_says_srgb(exif: &[u8]) -> bool {
    color_space(tiff_body(exif)) == Some(SRGB)
}

fn color_space(b: &[u8]) -> Option<u16> {
    let (order, first, count, entry_bytes, big) = ifd0_entries(b)?;
    let exif_ifd = (0..count).find_map(|i| {
        let e = first.checked_add(i.checked_mul(entry_bytes)?)?;
        (order.u16(b, e)? == TAG_EXIF_IFD).then(|| pointer(order, b, e, big)).flatten()
    })?;
    let (count, first) = if big {
        (usize::try_from(order.u64(b, exif_ifd)?).ok()?, exif_ifd.checked_add(8)?)
    } else {
        (usize::from(order.u16(b, exif_ifd)?), exif_ifd.checked_add(2)?)
    };
    // A count larger than the block can hold is damage; the loop stops at the first entry
    // outside it anyway.
    (0..count.min(b.len() / entry_bytes.max(1))).find_map(|i| {
        let e = first.checked_add(i.checked_mul(entry_bytes)?)?;
        if order.u16(b, e)? != TAG_COLOR_SPACE {
            return None;
        }
        let ty = order.u16(b, e.checked_add(2)?)?;
        let n = if big { order.u64(b, e.checked_add(4)?)? } else { u64::from(order.u32(b, e.checked_add(4)?)?) };
        let value = e.checked_add(if big { 12 } else { 8 })?;
        // The spec says one SHORT; inline either way.
        match (ty, n) {
            (TYPE_SHORT, 1) => order.u16(b, value),
            (TYPE_LONG, 1) => order.u32(b, value).and_then(|v| u16::try_from(v).ok()),
            _ => None,
        }
    })
}

/// The Exif IFD offset an ExifIFDPointer entry holds (one LONG or IFD; BigTIFF: LONG8 or IFD8).
fn pointer(order: Order, b: &[u8], e: usize, big: bool) -> Option<usize> {
    let ty = order.u16(b, e.checked_add(2)?)?;
    let n = if big { order.u64(b, e.checked_add(4)?)? } else { u64::from(order.u32(b, e.checked_add(4)?)?) };
    if n != 1 {
        return None;
    }
    let at = match (big, ty) {
        (false, TYPE_LONG | TYPE_IFD) => u64::from(order.u32(b, e.checked_add(8)?)?),
        (true, TYPE_LONG | TYPE_IFD) => u64::from(order.u32(b, e.checked_add(12)?)?),
        (true, TYPE_LONG8 | TYPE_IFD8) => order.u64(b, e.checked_add(12)?)?,
        _ => return None,
    };
    let at = usize::try_from(at).ok()?;
    (at >= 8 && at < b.len()).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little- or big-endian EXIF block: IFD0 with an ExifIFDPointer, then an Exif IFD
    /// holding ColorSpace = `cs` as one `ty`.
    fn exif(le: bool, ty: u16, cs: u16) -> Vec<u8> {
        let w16 = |v: &mut Vec<u8>, x: u16| v.extend(if le { x.to_le_bytes() } else { x.to_be_bytes() });
        let w32 = |v: &mut Vec<u8>, x: u32| v.extend(if le { x.to_le_bytes() } else { x.to_be_bytes() });
        let mut v = b"Exif\0\0".to_vec();
        let mut t = Vec::new();
        t.extend(if le { *b"II" } else { *b"MM" });
        w16(&mut t, 42);
        w32(&mut t, 8);
        // IFD0: one entry, the pointer to the Exif IFD at 8 + 2 + 12 + 4 = 26.
        w16(&mut t, 1);
        w16(&mut t, TAG_EXIF_IFD);
        w16(&mut t, TYPE_LONG);
        w32(&mut t, 1);
        w32(&mut t, 26);
        w32(&mut t, 0);
        // Exif IFD: ColorSpace.
        w16(&mut t, 1);
        w16(&mut t, TAG_COLOR_SPACE);
        w16(&mut t, ty);
        w32(&mut t, 1);
        if ty == TYPE_SHORT {
            w16(&mut t, cs);
            w16(&mut t, 0);
        } else {
            w32(&mut t, u32::from(cs));
        }
        w32(&mut t, 0);
        v.extend(t);
        v
    }

    #[test]
    fn reads_srgb_in_both_byte_orders() {
        for le in [true, false] {
            assert!(exif_says_srgb(&exif(le, TYPE_SHORT, 1)));
            assert!(exif_says_srgb(&exif(le, TYPE_LONG, 1)));
            assert!(!exif_says_srgb(&exif(le, TYPE_SHORT, 0xFFFF)), "uncalibrated");
            assert!(!exif_says_srgb(&exif(le, TYPE_SHORT, 2)));
        }
        // Without the JPEG prefix too.
        assert!(exif_says_srgb(exif(true, TYPE_SHORT, 1).get(6..).unwrap()));
    }

    #[test]
    fn malformed_blocks_are_not_srgb() {
        let good = exif(true, TYPE_SHORT, 1);
        // Cut anywhere before the ColorSpace value ends (its padding and the Exif IFD's
        // next-IFD pointer aren't needed to read it).
        for n in 0..good.len() - 6 {
            assert!(!exif_says_srgb(good.get(..n).unwrap()), "truncated at {n}");
        }
        // A pointer into the header or past the end.
        for at in [0u32, 4, 10_000, u32::MAX] {
            let mut b = good.clone();
            b.splice(6 + 18..6 + 22, at.to_le_bytes());
            assert!(!exif_says_srgb(&b), "pointer {at}");
        }
        // A huge entry count.
        let mut b = good.clone();
        b.splice(6 + 26..6 + 28, u16::MAX.to_le_bytes());
        let _ = exif_says_srgb(&b);
        assert!(!exif_says_srgb(b""));
        assert!(!exif_says_srgb(&[0xFF; 64]));
    }
}
