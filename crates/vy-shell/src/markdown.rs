//! The subset of Markdown the `fallback` text is shown with (SPEC-8.3.1):
//! headings (`#` to `###`), paragraphs, bullet lists (`-`, `*`) and
//! numbered lists; everything else is plain text. No HTML, ever
//! (SPEC-1.1.1): a tag is just characters.

/// A block of the fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A heading of level 1..=3.
    Heading {
        /// 1 to 3.
        level: u8,
        /// Text.
        text: String,
    },
    /// A paragraph; soft line breaks joined with spaces.
    Paragraph(String),
    /// A list item, with its marker (`•` or `1.`).
    Item {
        /// What to draw before the text.
        marker: String,
        /// Text.
        text: String,
    },
}

/// Parse `text` into blocks. Inline emphasis markers (`*`, `_`, `` ` ``)
/// are left as they are.
pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut number = 0u32;
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(paragraph.join(" ")));
            paragraph.clear();
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut paragraph, &mut blocks);
            number = 0;
            continue;
        }
        let hashes = line.bytes().take_while(|b| *b == b'#').count();
        if (1..=3).contains(&hashes) && line[hashes..].starts_with(' ') {
            flush(&mut paragraph, &mut blocks);
            number = 0;
            blocks.push(Block::Heading {
                level: hashes as u8,
                text: line[hashes..]
                    .trim()
                    .trim_end_matches('#')
                    .trim()
                    .to_string(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item {
                marker: "•".into(),
                text: rest.trim().to_string(),
            });
            continue;
        }
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && line[digits..].starts_with(". ") {
            flush(&mut paragraph, &mut blocks);
            number += 1;
            blocks.push(Block::Item {
                marker: format!("{number}."),
                text: line[digits + 2..].trim().to_string(),
            });
            continue;
        }
        paragraph.push(line);
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-8.3.1, SPEC-1.1.1: headings, paragraphs and lists come out as
    /// blocks; HTML stays text.
    #[test]
    fn spec_8_3_1_fallback_markdown_subset() {
        let text = "# Hej från Vy\n\nDen här sidan är en\nWebAssembly-modul.\n\n## Lista\n- ett\n* två\n\n1. a\n2. b\n#### inte rubrik\n<b>tag</b>\n";
        assert_eq!(
            parse(text),
            vec![
                Block::Heading {
                    level: 1,
                    text: "Hej från Vy".into()
                },
                Block::Paragraph("Den här sidan är en WebAssembly-modul.".into()),
                Block::Heading {
                    level: 2,
                    text: "Lista".into()
                },
                Block::Item {
                    marker: "•".into(),
                    text: "ett".into()
                },
                Block::Item {
                    marker: "•".into(),
                    text: "två".into()
                },
                Block::Item {
                    marker: "1.".into(),
                    text: "a".into()
                },
                Block::Item {
                    marker: "2.".into(),
                    text: "b".into()
                },
                Block::Paragraph("#### inte rubrik <b>tag</b>".into()),
            ]
        );
        assert_eq!(parse(""), vec![]);
        assert_eq!(parse("#nospace"), vec![Block::Paragraph("#nospace".into())]);
    }
}
