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

/// The `<img>` of a preview rendered by `vy-cli` at the URL path `src`.
fn preview_img(src: &str) -> String {
    format!(
        "<img src=\"{}\" alt=\"Förhandsvisning renderad av vy-cli\" loading=\"lazy\">\n",
        escape(src)
    )
}

/// The whole fallback page for a package at `vy_address`; `preview` is
/// the URL path of its preview image when one exists.
pub fn page(
    manifest: &Manifest,
    vy_address: &str,
    identity: &Hash,
    preview: Option<&str>,
) -> String {
    let body = match &manifest.fallback {
        Some(md) => markdown(md),
        None => format!(
            "<h1>{}</h1>\n<p>{}</p>\n",
            escape(&manifest.name),
            escape(&manifest.summary)
        ),
    };
    let preview = preview.map(preview_img).unwrap_or_default();
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n\
         <style>body{{font-family:system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;line-height:1.5}}\
         .open{{border:1px solid #888;border-radius:.5rem;padding:.75rem 1rem;word-break:break-all}}\
         img{{display:block;width:100%;height:auto;margin:1rem 0;border:1px solid #ccc;border-radius:.25rem}}\
         footer{{margin-top:3rem;color:#555;font-size:.85rem;word-break:break-all}}</style>\n\
         </head>\n<body>\n<main>\n\
         <p class=\"open\">Öppna i Vy: <a href=\"{addr}\">{addr}</a></p>\n\
         {preview}\
         {body}\
         <footer>\n<p>Det här är en Vy-sida: en WebAssembly-modul som ritar sig själv i en Vy-klient. \
         Webbläsaren visar reservtexten ur manifestet.</p>\n\
         <p>Paket: {identity}</p>\n<p><a href=\"/\">Alla sidor</a></p>\n</footer>\n</main>\n</body>\n</html>\n",
        lang = escape(&manifest.lang),
        title = escape(&manifest.name),
        addr = escape(vy_address),
        identity = identity,
    )
}

/// One package on the index page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Package name: the request path without the leading slash.
    pub name: &'a str,
    /// The manifest, for its `name`, `summary` and `lang`.
    pub manifest: &'a Manifest,
    /// URL path of the preview image when one exists.
    pub preview: Option<String>,
}

/// The index page: every package with its name, summary, "open in Vy"
/// link and preview, for the host `host` of the `vy://` links.
pub fn index(host: &str, entries: &[Entry<'_>]) -> String {
    let mut items = String::new();
    for e in entries {
        let href = format!("/{}", e.name);
        let addr = escape(&format!("vy://{host}{href}"));
        let preview = e
            .preview
            .as_deref()
            .map(|src| format!("<a href=\"{}\">{}</a>\n", escape(&href), preview_img(src)))
            .unwrap_or_default();
        items.push_str(&format!(
            "<li lang=\"{lang}\">\n{preview}<h2><a href=\"{href}\">{title}</a></h2>\n<p>{summary}</p>\n\
             <p class=\"open\">Öppna i Vy: <a href=\"{addr}\">{addr}</a></p>\n</li>\n",
            lang = escape(&e.manifest.lang),
            href = escape(&href),
            title = escape(&e.manifest.name),
            summary = escape(&e.manifest.summary),
        ));
    }
    format!(
        "<!doctype html>\n<html lang=\"sv\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Vy – sidor på {host}</title>\n\
         <style>body{{font-family:system-ui,sans-serif;max-width:64rem;margin:2rem auto;padding:0 1rem;line-height:1.5}}\
         .pages{{list-style:none;padding:0;display:grid;gap:1.5rem;grid-template-columns:repeat(auto-fill,minmax(18rem,1fr))}}\
         .pages li{{border:1px solid #ccc;border-radius:.5rem;padding:1rem}}\
         .pages img{{display:block;width:100%;height:auto;border:1px solid #ddd;border-radius:.25rem}}\
         .pages h2{{font-size:1.1rem;margin:.75rem 0 .25rem}}\
         .open{{word-break:break-all}}\
         footer{{margin-top:3rem;color:#555;font-size:.85rem}}</style>\n\
         </head>\n<body>\n<main>\n<h1>Vy – sidor på {host}</h1>\n\
         <p>{count} sidor. Varje sida är en WebAssembly-modul som ritar sig själv i en Vy-klient; \
         webbläsaren visar förhandsvisningen och reservtexten ur manifestet.</p>\n\
         <ul class=\"pages\">\n{items}</ul>\n\
         <footer>\n<p>Utgivarnyckel: <a href=\"/.well-known/vy-key\">/.well-known/vy-key</a></p>\n</footer>\n\
         </main>\n</body>\n</html>\n",
        host = escape(host),
        count = entries.len(),
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

    /// The index escapes every manifest field and the host, links each
    /// package both ways and shows a preview only where one exists.
    #[test]
    fn index_escapes_and_links() {
        let draft = vy_manifest::Draft::from_json(include_str!(
            "../../../conformance/manifests/hello.json"
        ))
        .unwrap();
        let mut hostile = draft.clone();
        hostile.name = "<b>Namn</b> & \"citat\"".into();
        hostile.summary = "<script>alert(1)</script>".into();
        let plain = draft.finish(&[], &[]);
        let hostile = hostile.finish(&[], &[]);
        let entries = [
            Entry {
                name: "hello",
                manifest: &plain,
                preview: Some("/_previews/hello.png".into()),
            },
            Entry {
                name: "docs/x",
                manifest: &hostile,
                preview: None,
            },
        ];
        let html = index("vy.example:8443", &entries);
        assert!(html.contains("<title>Vy – sidor på vy.example:8443</title>"));
        assert!(html.contains("2 sidor"));
        assert!(html.contains("<h2><a href=\"/hello\">Hej från Vy</a></h2>"));
        assert!(html.contains("href=\"vy://vy.example:8443/hello\""));
        assert!(html.contains("href=\"vy://vy.example:8443/docs/x\""));
        assert!(html.contains("<a href=\"/hello\"><img src=\"/_previews/hello.png\""));
        assert_eq!(html.matches("<img").count(), 1, "no preview for docs/x");
        assert!(html.contains("&lt;b&gt;Namn&lt;/b&gt; &amp; &quot;citat&quot;"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b>"));
        let page = page(&plain, "vy://vy.example/hello", &plain.identity(), None);
        assert!(!page.contains("<img"));
        let page = page_with_preview(&plain);
        assert!(page.contains("<img src=\"/_previews/hello.png\""));
    }

    fn page_with_preview(m: &Manifest) -> String {
        page(
            m,
            "vy://vy.example/hello",
            &m.identity(),
            Some("/_previews/hello.png"),
        )
    }
}
