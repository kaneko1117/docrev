use super::styles::{builtin_format, parse_cell_styles, parse_styles};
use super::theme::{apply_tint, default_palette, parse_hex_rgb, parse_theme_palette};
use super::worksheet::parse_pane;
use super::*;

fn formats_of(styles: &[CellStyle]) -> Vec<Option<&str>> {
    styles.iter().map(|s| s.format.as_deref()).collect()
}

#[test]
fn a_frozen_pane_yields_row_and_col_counts() {
    let xml = r#"<worksheet><sheetViews>
        <sheetView workbookViewId="0">
            <pane xSplit="1" ySplit="2" topLeftCell="B3" state="frozen"/>
        </sheetView>
    </sheetViews><sheetData/></worksheet>"#;
    assert_eq!(parse_pane(xml).unwrap(), Some((2, 1)));
}

#[test]
fn frozen_split_counts_too_and_axes_may_be_absent() {
    let rows_only = r#"<sheetView><pane ySplit="1" state="frozenSplit"/></sheetView>"#;
    assert_eq!(parse_pane(rows_only).unwrap(), Some((1, 0)));
    let cols_only = r#"<sheetView><pane xSplit="3" state="frozen"/></sheetView>"#;
    assert_eq!(parse_pane(cols_only).unwrap(), Some((0, 3)));
}

#[test]
fn non_frozen_splits_and_garbage_are_ignored() {
    // a plain split is in twips
    let split = r#"<sheetView><pane xSplit="2310" ySplit="1050"/></sheetView>"#;
    assert_eq!(parse_pane(split).unwrap(), None);
    let garbage = r#"<sheetView><pane xSplit="NaN" ySplit="-3" state="frozen"/></sheetView>"#;
    assert_eq!(parse_pane(garbage).unwrap(), None);
    let none = r"<worksheet><sheetData/></worksheet>";
    assert_eq!(parse_pane(none).unwrap(), None);
}

#[test]
fn styles_resolve_custom_and_builtin_ids() {
    let styles = r##"<styleSheet>
        <numFmts count="1">
            <numFmt numFmtId="164" formatCode="#,##0&quot;千円&quot;"/>
        </numFmts>
        <cellXfs count="4">
            <xf numFmtId="0"/>
            <xf numFmtId="9"/>
            <xf numFmtId="164"/>
            <xf numFmtId="999"/>
        </cellXfs>
    </styleSheet>"##;
    let styles = parse_styles(styles, &default_palette()).unwrap();
    assert_eq!(
        formats_of(&styles),
        vec![None, Some("0%"), Some("#,##0\"千円\""), None]
    );
    assert!(styles.iter().all(|s| s.fill.is_none()));
}

#[test]
fn styles_ignore_cell_style_xfs_and_dxfs() {
    let styles = r#"<styleSheet>
        <dxfs count="1">
            <numFmt numFmtId="164" formatCode="0.000"/>
            <fill><patternFill patternType="solid"><fgColor rgb="FF123456"/></patternFill></fill>
        </dxfs>
        <cellStyleXfs count="1"><xf numFmtId="9"/></cellStyleXfs>
        <cellXfs count="1"><xf numFmtId="3"/></cellXfs>
    </styleSheet>"#;
    let styles = parse_styles(styles, &default_palette()).unwrap();
    assert_eq!(formats_of(&styles), vec![Some("#,##0")]);
    assert_eq!(styles[0].fill, None);
}

#[test]
fn solid_fills_resolve_rgb_theme_and_tint() {
    let styles = r#"<styleSheet>
        <fills count="5">
            <fill><patternFill/></fill>
            <fill><patternFill patternType="gray125"/></fill>
            <fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/><bgColor indexed="64"/></patternFill></fill>
            <fill><patternFill patternType="solid"><fgColor theme="4"/></patternFill></fill>
            <fill><patternFill patternType="solid"><fgColor theme="4" tint="0.5"/></patternFill></fill>
        </fills>
        <cellXfs count="5">
            <xf fillId="0"/>
            <xf fillId="1"/>
            <xf fillId="2"/>
            <xf fillId="3"/>
            <xf fillId="4"/>
        </cellXfs>
    </styleSheet>"#;
    let styles = parse_styles(styles, &default_palette()).unwrap();
    let fills: Vec<_> = styles.iter().map(|s| s.fill).collect();
    // accent1 #4472C4; tint 0.5 lightens each channel halfway to white
    assert_eq!(
        fills,
        vec![
            None,
            None,
            Some((0xFF, 0x00, 0x00)),
            Some((0x44, 0x72, 0xC4)),
            Some((162, 185, 226)),
        ]
    );
}

#[test]
fn theme_palette_reads_clr_scheme_in_display_order() {
    let theme = r#"<a:theme xmlns:a="x"><a:themeElements><a:clrScheme name="Office">
        <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
        <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
        <a:dk2><a:srgbClr val="44546A"/></a:dk2>
        <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
        <a:accent1><a:srgbClr val="112233"/></a:accent1>
        <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
        <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
        <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
        <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
        <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
        <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
        <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme></a:themeElements></a:theme>"#;
    let palette = parse_theme_palette(theme).unwrap();
    // 0=lt1 (white), 1=dk1 (black)
    assert_eq!(palette[0], (0xFF, 0xFF, 0xFF));
    assert_eq!(palette[1], (0x00, 0x00, 0x00));
    assert_eq!(palette[4], (0x11, 0x22, 0x33), "accent1 from the file");
}

#[test]
fn truncated_theme_is_an_error() {
    let theme = r#"<a:theme xmlns:a="x"><a:clrScheme>
        <a:dk1><a:srgbClr val="000000"/></a:dk1>
    </a:clrScheme></a:theme>"#;
    assert!(parse_theme_palette(theme).is_err());
}

#[test]
fn multibyte_hex_strings_are_rejected_not_a_panic() {
    // "Aあ12" is 6 bytes; slicing at byte 2 would split あ
    assert_eq!(parse_hex_rgb("Aあ12"), None);
    assert_eq!(parse_hex_rgb("あdd12"), None, "8-byte variant");
    assert_eq!(parse_hex_rgb("FF0000"), Some((0xFF, 0, 0)));
    assert_eq!(parse_hex_rgb("00FF0000"), Some((0xFF, 0, 0)));
    assert_eq!(parse_hex_rgb("ZZ0000"), None, "non-hex ASCII");
}

#[test]
fn an_empty_palette_drops_theme_fills_but_keeps_rgb() {
    let styles = r#"<styleSheet>
        <fills count="3">
            <fill><patternFill/></fill>
            <fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill>
            <fill><patternFill patternType="solid"><fgColor theme="4"/></patternFill></fill>
        </fills>
        <cellXfs count="2">
            <xf fillId="1"/>
            <xf fillId="2"/>
        </cellXfs>
    </styleSheet>"#;
    let styles = parse_styles(styles, &[]).unwrap();
    assert_eq!(styles[0].fill, Some((0xFF, 0x00, 0x00)));
    assert_eq!(styles[1].fill, None, "unresolvable theme paints nothing");
}

#[test]
fn the_default_font_is_skipped_by_id_not_by_color() {
    let styles = r#"<styleSheet>
        <fonts count="2">
            <font><color rgb="FF0D0D0D"/></font>
            <font><color rgb="FFFF0000"/></font>
        </fonts>
        <cellXfs count="2">
            <xf fontId="0"/>
            <xf fontId="1"/>
        </cellXfs>
    </styleSheet>"#;
    let styles = parse_styles(styles, &default_palette()).unwrap();
    assert_eq!(styles[0].font, None, "font 0 is never an author's choice");
    assert_eq!(styles[1].font, Some((0xFF, 0x00, 0x00)));
}

#[test]
fn multibyte_font_colors_are_rejected_not_a_panic() {
    let styles = r#"<styleSheet>
        <fonts count="2">
            <font/>
            <font><color rgb="Aあ12"/></font>
        </fonts>
        <cellXfs count="1">
            <xf fontId="1"/>
        </cellXfs>
    </styleSheet>"#;
    let styles = parse_styles(styles, &default_palette()).unwrap();
    assert_eq!(styles[0].font, None);
}

#[test]
fn tint_lightens_and_darkens() {
    assert_eq!(apply_tint((100, 100, 100), 0.0), (100, 100, 100));
    assert_eq!(apply_tint((100, 200, 0), 0.5), (178, 228, 128));
    assert_eq!(apply_tint((100, 200, 0), -0.5), (50, 100, 0));
    assert_eq!(apply_tint((10, 10, 10), 5.0), (255, 255, 255), "clamped");
}

#[test]
fn date_builtins_resolve_to_their_japanese_renderings() {
    for id in 14..=22 {
        assert!(builtin_format(id).is_some(), "id {id} must resolve");
    }
    assert_eq!(builtin_format(14), Some("yyyy/m/d"));
    assert_eq!(builtin_format(28), Some("[$-411]ggge\"年\"m\"月\"d\"日\""));
    assert_eq!(builtin_format(46), Some("[h]:mm:ss"));
    assert_eq!(builtin_format(47), None, "fractional seconds stay out");
    assert_eq!(builtin_format(48), None, "scientific stays unresolved");
    assert_eq!(builtin_format(49), None, "text format must stay unresolved");
}

fn percent_style() -> Vec<CellStyle> {
    vec![
        CellStyle::default(),
        CellStyle {
            format: Some("0%".to_string()),
            fill: None,
            font: None,
        },
    ]
}

#[test]
fn cell_styles_keep_only_cells_with_a_visible_style() {
    let sheet = r#"<worksheet><sheetData>
        <row r="1">
            <c r="A1" s="1"><v>0.15</v></c>
            <c r="B1" s="0"><v>1</v></c>
            <c r="C1"><v>2</v></c>
            <c r="D1" s="1"><v>0.25</v></c>
        </row>
    </sheetData></worksheet>"#;
    let cells = parse_cell_styles(sheet, &percent_style()).unwrap();
    assert_eq!(cells.get(&(0, 0)), Some(&1));
    assert_eq!(cells.get(&(0, 3)), Some(&1));
    assert!(!cells.contains_key(&(0, 1)), "plain style is dropped");
    assert!(!cells.contains_key(&(0, 2)), "no style at all");
}

#[test]
fn cells_without_references_take_sequential_positions() {
    let sheet = r#"<worksheet><sheetData>
        <row><c s="1"><v>1</v></c><c><v>2</v></c><c s="1"><v>3</v></c></row>
        <row r="5"><c s="1"><v>4</v></c></row>
        <row><c r="B6" s="1"/><c s="1"/></row>
    </sheetData></worksheet>"#;
    let cells = parse_cell_styles(sheet, &percent_style()).unwrap();
    let positions: Vec<(u32, u32)> = {
        let mut p: Vec<_> = cells.keys().copied().collect();
        p.sort_unstable();
        p
    };
    // r="5" is row index 4; B6 re-anchors to (5, 1) and the next cell continues at (5, 2)
    assert_eq!(positions, vec![(0, 0), (0, 2), (4, 0), (5, 1), (5, 2)]);
}

#[test]
fn out_of_range_style_indices_are_ignored() {
    let sheet = r#"<worksheet><sheetData>
        <row r="1"><c r="A1" s="7"><v>1</v></c></row>
    </sheetData></worksheet>"#;
    let cells = parse_cell_styles(sheet, &percent_style()).unwrap();
    assert!(cells.is_empty());
}
