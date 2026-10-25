//! Exports a table from the UI (the connections page) as Excel, CSV, JSON,
//! HTML or Markdown. The Excel file is a minimal SpreadsheetML package:
//! one sheet, bold frozen header with a filter, numbers as numbers.

use std::io::Write;

use anyhow::{bail, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Cell {
    Num(f64),
    Text(String),
}

impl Cell {
    fn text(&self) -> String {
        match self {
            Cell::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", *n as i64),
            Cell::Num(n) => n.to_string(),
            Cell::Text(s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

pub const FORMATS: [&str; 5] = ["xlsx", "csv", "json", "html", "md"];

pub fn render(t: &Table, format: &str) -> Result<Vec<u8>> {
    Ok(match format {
        "xlsx" => xlsx(t)?,
        "csv" => csv(t).into_bytes(),
        "json" => json(t)?,
        "html" => html(t).into_bytes(),
        "md" => markdown(t).into_bytes(),
        f => bail!("不支持的导出格式 {f}"),
    })
}

/// `prism-<what>-<YYYYmmdd-HHMMSS>.<ext>`
pub fn file_name(what: &str, format: &str) -> String {
    let what: String = what.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    format!("prism-{what}-{}.{format}", chrono::Local::now().format("%Y%m%d-%H%M%S"))
}

/// Spreadsheet apps run cells starting with these as formulas.
fn defuse(s: &str) -> String {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    }
}

/// UTF-8 with BOM so Excel shows Chinese correctly.
fn csv(t: &Table) -> String {
    let field = |s: String| {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s
        }
    };
    let mut out = String::from("\u{feff}");
    out.push_str(&t.headers.iter().map(|h| field(h.clone())).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    for r in &t.rows {
        let cells: Vec<String> = r
            .iter()
            .map(|c| match c {
                Cell::Text(s) => field(defuse(s)),
                n => n.text(),
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

fn json(t: &Table) -> Result<Vec<u8>> {
    let rows: Vec<serde_json::Map<String, serde_json::Value>> = t
        .rows
        .iter()
        .map(|r| {
            t.headers
                .iter()
                .zip(r)
                .map(|(h, c)| {
                    let v = match c {
                        Cell::Num(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
                        Cell::Text(s) => serde_json::Value::String(s.clone()),
                    };
                    (h.clone(), v)
                })
                .collect()
        })
        .collect();
    Ok(serde_json::to_vec_pretty(&rows)?)
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // Not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn html(t: &Table) -> String {
    let mut o = String::from(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Prism</title>\n<style>\
         body{font:13px system-ui,sans-serif;margin:16px}table{border-collapse:collapse}\
         th,td{border:1px solid #ccc;padding:4px 8px;text-align:left;white-space:nowrap}\
         th{background:#f0f2f8}td.n{text-align:right}</style></head><body>\n<table>\n<tr>",
    );
    for h in &t.headers {
        o.push_str(&format!("<th>{}</th>", esc(h)));
    }
    o.push_str("</tr>\n");
    for r in &t.rows {
        o.push_str("<tr>");
        for c in r {
            match c {
                Cell::Num(_) => o.push_str(&format!("<td class=\"n\">{}</td>", c.text())),
                Cell::Text(s) => o.push_str(&format!("<td>{}</td>", esc(s))),
            }
        }
        o.push_str("</tr>\n");
    }
    o.push_str("</table>\n</body></html>\n");
    o
}

fn markdown(t: &Table) -> String {
    let cell = |s: String| s.replace('|', "\\|").replace(['\r', '\n'], " ");
    let mut o = format!("| {} |\n", t.headers.iter().map(|h| cell(h.clone())).collect::<Vec<_>>().join(" | "));
    o.push_str(&format!("|{}\n", "---|".repeat(t.headers.len())));
    for r in &t.rows {
        o.push_str(&format!("| {} |\n", r.iter().map(|c| cell(c.text())).collect::<Vec<_>>().join(" | ")));
    }
    o
}

/// A1-style column name: 0 -> A, 26 -> AA.
fn col(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

fn xlsx(t: &Table) -> Result<Vec<u8>> {
    let ncols = t.headers.len().max(1);
    let last = format!("{}{}", col(ncols - 1), t.rows.len() + 1);
    // Column widths from the longest value (CJK characters count double).
    let width = |s: &str| s.chars().map(|c| if (c as u32) > 0x2e80 { 2 } else { 1 }).sum::<usize>();
    let mut widths: Vec<usize> = t.headers.iter().map(|h| width(h)).collect();
    widths.resize(ncols, 8);
    for r in &t.rows {
        for (i, c) in r.iter().enumerate().take(ncols) {
            widths[i] = widths[i].max(width(&c.text()));
        }
    }

    let mut sheet = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews><cols>",
    );
    for (i, w) in widths.iter().enumerate() {
        sheet.push_str(&format!("<col min=\"{n}\" max=\"{n}\" width=\"{}\" customWidth=\"1\"/>", (w + 2).min(80), n = i + 1));
    }
    sheet.push_str("</cols><sheetData>");
    let text_cell = |r: usize, c: usize, s: &str, style: u8| {
        format!("<c r=\"{}{r}\" t=\"inlineStr\" s=\"{style}\"><is><t xml:space=\"preserve\">{}</t></is></c>", col(c), esc(s))
    };
    sheet.push_str("<row r=\"1\">");
    for (i, h) in t.headers.iter().enumerate() {
        sheet.push_str(&text_cell(1, i, h, 1));
    }
    sheet.push_str("</row>");
    for (ri, r) in t.rows.iter().enumerate() {
        let rn = ri + 2;
        sheet.push_str(&format!("<row r=\"{rn}\">"));
        for (ci, c) in r.iter().enumerate() {
            match c {
                Cell::Num(n) if n.is_finite() => sheet.push_str(&format!("<c r=\"{}{rn}\"><v>{n}</v></c>", col(ci))),
                Cell::Num(_) => {}
                Cell::Text(s) => sheet.push_str(&text_cell(rn, ci, s, 0)),
            }
        }
        sheet.push_str("</row>");
    }
    sheet.push_str(&format!("</sheetData><autoFilter ref=\"A1:{last}\"/></worksheet>"));

    let files: [(&str, String); 6] = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
             <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
             <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
             </Types>"
                .into(),
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
             </Relationships>"
                .into(),
        ),
        (
            "xl/workbook.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <sheets><sheet name=\"Prism\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
             <definedNames><definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">Prism!$A$1:$"
                .to_string()
                + &format!("{}${}", col(ncols - 1), t.rows.len() + 1)
                + "</definedName></definedNames></workbook>",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
             </Relationships>"
                .into(),
        ),
        (
            "xl/styles.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
             <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
             <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
             <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
             <cellXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
             <xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/></cellXfs>\
             <cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
             </styleSheet>"
                .into(),
        ),
        ("xl/worksheets/sheet1.xml", sheet),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in files {
        zip.start_file(name, opts)?;
        zip.write_all(body.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn table() -> Table {
        serde_json::from_str(
            r#"{"headers": ["主机", "程序", "上传（字节）"],
                "rows": [["a.com:443", "chrome.exe", 1234], ["=cmd|' /c calc'!A1", "x,\"y\"", 0.5], ["<b>&", "", 0]]}"#,
        )
        .unwrap()
    }

    #[test]
    fn cells_parse_as_numbers_or_text() {
        let t = table();
        assert_eq!(t.rows[0][2], Cell::Num(1234.0));
        assert_eq!(t.rows[0][2].text(), "1234");
        assert_eq!(t.rows[1][2].text(), "0.5");
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let s = csv(&table());
        assert!(s.starts_with("\u{feff}主机,程序,上传（字节）\r\n"));
        assert!(s.contains("a.com:443,chrome.exe,1234\r\n"));
        assert!(s.contains("'=cmd|' /c calc'!A1,\"x,\"\"y\"\"\",0.5\r\n"), "{s}");
    }

    #[test]
    fn json_html_markdown() {
        let t = table();
        let v: serde_json::Value = serde_json::from_slice(&json(&t).unwrap()).unwrap();
        assert_eq!(v[0]["主机"], "a.com:443");
        assert_eq!(v[0]["上传（字节）"], 1234.0);
        let h = html(&t);
        assert!(h.contains("<td>&lt;b&gt;&amp;</td>") && h.contains("<td class=\"n\">1234</td>"));
        let m = markdown(&t);
        assert_eq!(m.lines().nth(1), Some("|---|---|---|"));
        assert!(m.contains("| =cmd\\|' /c calc'!A1 |"));
        assert!(render(&t, "pdf").is_err());
    }

    #[test]
    fn xlsx_package() {
        assert_eq!((col(0), col(25), col(26), col(27), col(701), col(702)), ("A".into(), "Z".into(), "AA".into(), "AB".into(), "ZZ".into(), "AAA".into()));
        let data = xlsx(&table()).unwrap();
        // For checking with other spreadsheet readers.
        if let Ok(p) = std::env::var("PRISM_XLSX_OUT") {
            std::fs::write(p, &data).unwrap();
        }
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        for name in ["[Content_Types].xml", "_rels/.rels", "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "xl/styles.xml"] {
            assert!(z.by_name(name).is_ok(), "{name}");
        }
        let mut sheet = String::new();
        z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        assert!(sheet.contains("<c r=\"A1\" t=\"inlineStr\" s=\"1\"><is><t xml:space=\"preserve\">主机</t></is></c>"));
        assert!(sheet.contains("<c r=\"C2\"><v>1234</v></c>"));
        assert!(sheet.contains("<t xml:space=\"preserve\">&lt;b&gt;&amp;</t>"));
        assert!(sheet.contains("<autoFilter ref=\"A1:C4\"/>"));
    }
}
