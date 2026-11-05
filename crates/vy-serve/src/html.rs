//! HTML fallback for ordinary browsers: the manifest's `fallback` Markdown
//! rendered with headings and paragraphs only, an "open in Vy" link and
//! the package identity. No other HTML is ever produced by the server.

use vy_manifest::{Hash, Manifest};

/// Escape text for an HTML body or attribute.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Render the Markdown subset: `#`–`######` headings and paragraphs
/// separated by blank lines. Everything else is literal text.
pub fn markdown(md: &str) -> String {
    let mut html = String::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, html: &mut String| {
        if !paragraph.is_empty() {
            html.push_str("<p>");
            html.push_str(&escape(&paragraph.join(" ")));
            html.push_str("</p>\n");
            paragraph.clear();
        }
    };
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut html);
            continue;
        }
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            flush(&mut paragraph, &mut html);
            let text = trimmed[hashes..].trim();
            html.push_str(&format!("<h{hashes}>{}</h{hashes}>\n", escape(text)));
        } else {
            paragraph.push(trimmed);
        }
    }
    flush(&mut paragraph, &mut html);
    html
}

/// The whole fallback page for a package at `vy_address`.
pub fn page(manifest: &Manifest, vy_address: &str, identity: &Hash) -> String {
    let body = match &manifest.fallback {
        Some(md) => markdown(md),
        None => format!(
            "<h1>{}</h1>\n<p>{}</p>\n",
            escape(&manifest.name),
            escape(&manifest.summary)
        ),
    };
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n\
         <style>body{{font-family:system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;line-height:1.5}}\
         .open{{border:1px solid #888;border-radius:.5rem;padding:.75rem 1rem}}\
         footer{{margin-top:3rem;color:#555;font-size:.85rem;word-break:break-all}}</style>\n\
         </head>\n<body>\n<main>\n\
         <p class=\"open\">Öppna i Vy: <a href=\"{addr}\">{addr}</a></p>\n\
         {body}\
         <footer>\n<p>Det här är en Vy-sida: en WebAssembly-modul som ritar sig själv i en Vy-klient. \
         Webbläsaren visar reservtexten ur manifestet.</p>\n\
         <p>Paket: {identity}</p>\n</footer>\n</main>\n</body>\n</html>\n",
        lang = escape(&manifest.lang),
        title = escape(&manifest.name),
        addr = escape(vy_address),
        identity = identity,
    )
}

/// A short HTML page for an address that has no package.
pub fn not_found(path: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"sv\"><head><meta charset=\"utf-8\"><title>Ingen sida</title></head>\n\
         <body><main><h1>Ingen sida här</h1><p>Det finns inget Vy-paket på <code>{}</code>.</p></main></body></html>\n",
        escape(path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_subset() {
        let html = markdown("# Rubrik\n\nEtt stycke\npå två rader.\n\n## Under <b>\n\nSista.");
        assert_eq!(
            html,
            "<h1>Rubrik</h1>\n<p>Ett stycke på två rader.</p>\n<h2>Under &lt;b&gt;</h2>\n<p>Sista.</p>\n"
        );
        assert_eq!(markdown("#hashtag"), "<p>#hashtag</p>\n");
        assert_eq!(markdown("   "), "");
    }

    #[test]
    fn escapes_everything() {
        assert_eq!(
            escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert!(not_found("/<script>").contains("&lt;script&gt;"));
    }
}
