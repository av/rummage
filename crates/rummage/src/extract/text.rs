//! Text, markdown, code, CSV, JSON and HTML: about 1k-token passages with overlap.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rummage_core::EmbedInput;
use rummage_core::store::ChunkRow;
use rummage_core::types::SegmentKind;

use super::{Meta, Outcome, Piece, Sink, ext};

/// Characters per passage: about 1k tokens at ~4 characters per token.
pub const CHUNK_CHARS: usize = 4000;
/// Characters repeated between neighbouring passages.
pub const OVERLAP_CHARS: usize = 400;
/// Only the first this-many bytes of a text file are indexed.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub fn extract(path: &Path, sink: &mut Sink) -> Result<Outcome> {
    let mut bytes = vec![];
    std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        bail!("looks like a binary file, not text");
    }
    let raw = String::from_utf8_lossy(&bytes);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (title, body) = match ext(path).as_str() {
        "html" | "htm" | "xhtml" => {
            let (t, b) = html_to_text(&raw);
            (t.unwrap_or_else(|| name.clone()), b)
        }
        "md" | "markdown" | "mdx" => (
            markdown_title(&raw).unwrap_or_else(|| name.clone()),
            raw.into_owned(),
        ),
        _ => (name.clone(), raw.into_owned()),
    };
    emit_passages(&title, &body, None, sink)?;
    Ok(Outcome::Done(Meta::default()))
}

/// Sends `body` as passages titled `title` (each with `page`, if given). An empty body still
/// sends one passage, so the file can be found by its name.
pub fn emit_passages(title: &str, body: &str, page: Option<i64>, sink: &mut Sink) -> Result<()> {
    let mut chunks = chunk(body, CHUNK_CHARS, OVERLAP_CHARS);
    if chunks.is_empty() && page.is_none() {
        chunks.push(String::new());
    }
    for text in chunks {
        let embed_text = if text.is_empty() {
            title.to_string()
        } else {
            text.clone()
        };
        sink(Piece {
            chunk: ChunkRow {
                kind: SegmentKind::Text,
                page,
                text: Some(text),
                ..Default::default()
            },
            input: EmbedInput::Document {
                title: Some(title.to_string()),
                text: embed_text,
            },
        })?;
    }
    Ok(())
}

/// The first `# Heading` of a markdown file (after any front matter).
pub fn markdown_title(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let mut in_front = false;
    let mut first = true;
    for line in lines.by_ref() {
        let t = line.trim();
        if first && t == "---" {
            in_front = true;
            first = false;
            continue;
        }
        first = false;
        if in_front {
            if t == "---" {
                in_front = false;
            } else if let Some(v) = t.strip_prefix("title:") {
                let v = v.trim().trim_matches(['"', '\'']).trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
            continue;
        }
        if let Some(h) = t.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim();
            if !h.is_empty() {
                return Some(h.to_string());
            }
        }
    }
    None
}

/// Splits text into passages of about `size` characters that overlap by `overlap`, breaking at
/// paragraph, line, sentence or word boundaries where it can.
pub fn chunk(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec![];
    }
    // Byte offset of every char boundary, so positions can be counted in characters.
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let n = bounds.len() - 1;
    let mut out = vec![];
    let mut start = 0;
    while start < n {
        let mut end = (start + size).min(n);
        if end < n {
            // Prefer the latest good break in the second half of the window.
            let window = &text[bounds[start + size / 2]..bounds[end]];
            let cut = ["\n\n", "\n", ". ", " "]
                .iter()
                .find_map(|sep| window.rfind(sep).map(|p| p + sep.len()));
            if let Some(p) = cut {
                let byte = bounds[start + size / 2] + p;
                end = bounds.partition_point(|b| *b < byte);
            }
        }
        let piece = text[bounds[start]..bounds[end]].trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        if end >= n {
            break;
        }
        let next = end.saturating_sub(overlap).max(start + 1);
        // Start the overlap at a word.
        let back = &text[bounds[next]..bounds[end]];
        start = match back.find(char::is_whitespace) {
            Some(p) if p < back.len() / 2 => bounds.partition_point(|b| *b < bounds[next] + p + 1),
            _ => next,
        };
    }
    out
}

/// Visible text of an HTML page, and its `<title>`.
pub fn html_to_text(html: &str) -> (Option<String>, String) {
    let mut out = String::with_capacity(html.len() / 2);
    let mut title = None;
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < html.len() {
        if bytes[i] == b'<' {
            let rest = &lower[i..];
            // Skip whole script/style blocks and comments.
            let skip_to = if rest.starts_with("<!--") {
                rest.find("-->").map(|p| i + p + 3)
            } else if rest.starts_with("<script") {
                rest.find("</script>").map(|p| i + p + 9)
            } else if rest.starts_with("<style") {
                rest.find("</style>").map(|p| i + p + 8)
            } else if rest.starts_with("<title") {
                let open_end = rest.find('>').map(|p| i + p + 1);
                let close = rest.find("</title>").map(|p| i + p);
                if let (Some(a), Some(b)) = (open_end, close)
                    && a <= b
                {
                    let t = decode_entities(html[a..b].trim());
                    if !t.is_empty() {
                        title = Some(t);
                    }
                }
                close.map(|c| c + 8)
            } else {
                let block = [
                    "<p", "<br", "<div", "<li", "<h1", "<h2", "<h3", "<h4", "<tr", "</p",
                ]
                .iter()
                .any(|t| rest.starts_with(t));
                if block {
                    out.push('\n');
                }
                rest.find('>').map(|p| i + p + 1)
            };
            match skip_to {
                Some(j) => i = j,
                None => break,
            }
        } else {
            let next = html[i..].find('<').map(|p| i + p).unwrap_or(html.len());
            out.push_str(&decode_entities(&html[i..next]));
            i = next;
        }
    }
    // Collapse runs of blank space.
    let mut text = String::with_capacity(out.len());
    let mut blank_lines = 0;
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_lines += 1;
            if blank_lines == 1 && !text.is_empty() {
                text.push('\n');
            }
        } else {
            blank_lines = 0;
            text.push_str(&line);
            text.push('\n');
        }
    }
    (title, text.trim().to_string())
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_overlap_and_cover_everything() {
        let para = "word ".repeat(300);
        let text = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk(&text, 1000, 100);
        assert!(chunks.len() >= 4, "{}", chunks.len());
        assert!(chunks.iter().all(|c| c.chars().count() <= 1000));
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert!(total > text.trim().len(), "overlap repeats some text");
        assert_eq!(chunk("short", 1000, 100), vec!["short"]);
        assert!(chunk("  \n ", 1000, 100).is_empty());
        // Multi-byte text never splits inside a character.
        let cyr = "привет мир ".repeat(500);
        assert!(
            chunk(&cyr, 300, 30)
                .iter()
                .all(|c| c.starts_with('п') || c.starts_with('м'))
        );
    }

    #[test]
    fn markdown_titles() {
        assert_eq!(
            markdown_title("intro\n## Second\n# First").as_deref(),
            Some("Second")
        );
        assert_eq!(
            markdown_title("---\ntitle: \"Front\"\n---\n# H").as_deref(),
            Some("Front")
        );
        assert_eq!(
            markdown_title("---\ndate: x\n---\n# H").as_deref(),
            Some("H")
        );
        assert_eq!(markdown_title("no heading"), None);
    }

    #[test]
    fn html_text() {
        let (t, body) = html_to_text(
            "<html><head><title>My &amp; Page</title><style>p{}</style></head>\
             <body><script>var x;</script><p>Hello&nbsp;<b>world</b></p><!-- c --><p>Two &#x41;</p></body></html>",
        );
        assert_eq!(t.as_deref(), Some("My & Page"));
        assert_eq!(body, "Hello world\n\nTwo A");
    }
}
