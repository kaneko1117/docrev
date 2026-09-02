use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;

use super::MetaError;
use super::archive::attr_value;

/// The default Office palette, in `theme=` attribute order.
pub(super) fn default_palette() -> Vec<(u8, u8, u8)> {
    vec![
        (0xFF, 0xFF, 0xFF), // lt1
        (0x00, 0x00, 0x00), // dk1
        (0xE7, 0xE6, 0xE6), // lt2
        (0x44, 0x54, 0x6A), // dk2
        (0x44, 0x72, 0xC4), // accent1
        (0xED, 0x7D, 0x31), // accent2
        (0xA5, 0xA5, 0xA5), // accent3
        (0xFF, 0xC0, 0x00), // accent4
        (0x5B, 0x9B, 0xD5), // accent5
        (0x70, 0xAD, 0x47), // accent6
        (0x05, 0x63, 0xC1), // hlink
        (0x95, 0x4F, 0x72), // folHlink
    ]
}

/// `theme=` index order, which swaps each lt/dk pair relative to the file order.
const THEME_ORDER: [&[u8]; 12] = [
    b"lt1",
    b"dk1",
    b"lt2",
    b"dk2",
    b"accent1",
    b"accent2",
    b"accent3",
    b"accent4",
    b"accent5",
    b"accent6",
    b"hlink",
    b"folHlink",
];

/// `<a:clrScheme>` children hold `<a:srgbClr val=…/>` or `<a:sysClr lastClr=…/>`.
pub(super) fn parse_theme_palette(xml: &str) -> Result<Vec<(u8, u8, u8)>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut in_scheme = false;
    let mut current: Option<Vec<u8>> = None;
    let mut named: HashMap<Vec<u8>, (u8, u8, u8)> = HashMap::new();
    loop {
        match reader.read_event().map_err(|e| MetaError(e.to_string()))? {
            Event::Start(e) if e.local_name().as_ref() == b"clrScheme" => in_scheme = true,
            Event::End(e) if e.local_name().as_ref() == b"clrScheme" => break,
            Event::Start(e) if in_scheme && THEME_ORDER.contains(&e.local_name().as_ref()) => {
                current = Some(e.local_name().as_ref().to_vec());
            }
            Event::Start(e) | Event::Empty(e) if in_scheme => {
                let attr_name = match e.local_name().as_ref() {
                    b"srgbClr" => b"val".as_ref(),
                    b"sysClr" => b"lastClr".as_ref(),
                    _ => continue,
                };
                let Some(name) = current.clone() else {
                    continue;
                };
                let rgb = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == attr_name)
                    .and_then(|a| parse_hex_rgb(&attr_value(&a, reader.decoder())));
                if let Some(rgb) = rgb {
                    named.entry(name).or_insert(rgb);
                }
            }
            Event::End(e) if THEME_ORDER.contains(&e.local_name().as_ref()) => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    if named.len() < THEME_ORDER.len() {
        return Err(MetaError("incomplete clrScheme".to_string()));
    }
    Ok(THEME_ORDER.iter().map(|name| named[*name]).collect())
}

/// `"RRGGBB"` or `"AARRGGBB"` (alpha ignored).
pub(super) fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    // the byte slicing below panics mid-char on multi-byte input
    if !hex.is_ascii() {
        return None;
    }
    let rgb = match hex.len() {
        6 => hex,
        8 => &hex[2..],
        _ => return None,
    };
    let r = u8::from_str_radix(&rgb[0..2], 16).ok()?;
    let g = u8::from_str_radix(&rgb[2..4], 16).ok()?;
    let b = u8::from_str_radix(&rgb[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Positive lightens toward white, negative darkens toward black (linear approximation of Excel's HSL tint).
pub(super) fn apply_tint(rgb: (u8, u8, u8), tint: f64) -> (u8, u8, u8) {
    let tint = tint.clamp(-1.0, 1.0);
    let channel = |c: u8| -> u8 {
        let c = f64::from(c);
        let out = if tint >= 0.0 {
            c + (255.0 - c) * tint
        } else {
            c * (1.0 + tint)
        };
        out.round().clamp(0.0, 255.0) as u8
    };
    (channel(rgb.0), channel(rgb.1), channel(rgb.2))
}
