//! Native tests: the Markdown parser, and the page's preview layout,
//! links, tabs, caret following and storage against the SDK's mock host.

use super::*;
use md::{count, parse};
use vy_sdk::mock;

fn kinds(src: &str) -> Vec<Kind> {
    parse(src).blocks.iter().map(|b| b.kind).collect()
}

fn texts(src: &str) -> Vec<String> {
    parse(src).blocks.iter().map(|b| b.text.clone()).collect()
}

/// Every block the preview knows, from the lines that start it.
#[test]
fn blocks_are_recognised_by_their_first_characters() {
    let src = "# Ett\n## Två\n### Tre\n#### inte en rubrik\n\nStycke på\ntvå rader.\n\n\
               > Citat som\n> fortsätter\n\n- punkt\n  - under\n* stjärna\n1. första\n12. tolfte\n   3. inre\n\n\
               ---\n***\n\n```\nfn a() {\n\n    b\n}\n```\nefter";
    assert_eq!(
        kinds(src),
        [
            Kind::Heading(1),
            Kind::Heading(2),
            Kind::Heading(3),
            Kind::Para,
            Kind::Para,
            Kind::Quote,
            Kind::Bullet { nested: false },
            Kind::Bullet { nested: true },
            Kind::Bullet { nested: false },
            Kind::Number {
                nested: false,
                n: 1
            },
            Kind::Number {
                nested: false,
                n: 12
            },
            Kind::Number { nested: true, n: 3 },
            Kind::Rule,
            Kind::Rule,
            Kind::Code,
            Kind::Para,
        ]
    );
    let t = texts(src);
    assert_eq!(t[0], "Ett");
    assert_eq!(t[3], "#### inte en rubrik");
    assert_eq!(t[4], "Stycke på två rader.", "lines of a paragraph join");
    assert_eq!(t[5], "Citat som fortsätter");
    assert_eq!(t[7], "under");
    assert_eq!(
        t[14], "fn a() {\n\n    b\n}",
        "code keeps its lines and indent"
    );
    assert_eq!(t[15], "efter");

    // A plain line after a list item continues the item; a fence left
    // open runs to the end; an empty text has no blocks.
    assert_eq!(texts("- en\nfortsättning"), ["en fortsättning"]);
    assert_eq!(kinds("```\nkod"), [Kind::Code]);
    assert_eq!(texts("```\nkod"), ["kod"]);
    assert!(parse("").blocks.is_empty());
    assert!(parse("\n\n  \n").blocks.is_empty());
    assert_eq!(kinds("-- inte en linje"), [Kind::Para]);
    assert_eq!(kinds("1.utan mellanslag"), [Kind::Para]);
}

/// The source range of each block, for following the caret.
#[test]
fn blocks_know_where_they_came_from() {
    let src = "# Rubrik\n\nEtt stycke\nsom fortsätter.\n\n- punkt";
    let doc = parse(src);
    let ranges: Vec<&str> = doc.blocks.iter().map(|b| &src[b.src.0..b.src.1]).collect();
    assert_eq!(
        ranges,
        ["# Rubrik", "Ett stycke\nsom fortsätter.", "- punkt"]
    );
    assert_eq!(doc.block_at(0), Some(0));
    assert_eq!(
        doc.block_at(9),
        Some(0),
        "the blank line belongs to what is above"
    );
    assert_eq!(doc.block_at(15), Some(1));
    assert_eq!(doc.block_at(src.len()), Some(2));
    assert_eq!(parse("").block_at(0), None);
}

/// Inline markup becomes runs over the plain text.
#[test]
fn inline_markup_becomes_styled_runs() {
    let src = "a **b c** *d* _e_ `f*g` [h i](https://x.example/p) ***j***";
    let doc = parse(src);
    let b = &doc.blocks[0];
    assert_eq!(b.text, "a b c d e f*g h i j");
    let runs: Vec<(&str, u8, Option<u16>)> = b
        .runs
        .iter()
        .map(|r| (&b.text[r.start..r.end], r.style, r.link))
        .collect();
    assert_eq!(
        runs,
        [
            ("a ", 0, None),
            ("b c", BOLD, None),
            (" ", 0, None),
            ("d", ITALIC, None),
            (" ", 0, None),
            ("e", ITALIC, None),
            (" ", 0, None),
            ("f*g", CODE, None),
            (" ", 0, None),
            ("h i", 0, Some(0)),
            (" ", 0, None),
            ("j", BOLD | ITALIC, None),
        ]
    );
    assert_eq!(doc.links, ["https://x.example/p"]);
    // The runs cover the text, in order.
    let mut at = 0;
    for r in &b.runs {
        assert_eq!(r.start, at);
        at = r.end;
    }
    assert_eq!(at, b.text.len());

    // What is not markup stays as written.
    for (src, plain) in [
        (
            "snake_case_name och 2 * 3 * 4",
            "snake_case_name och 2 * 3 * 4",
        ),
        ("en ensam * stjärna", "en ensam * stjärna"),
        ("öppen **fetstil", "öppen **fetstil"),
        ("öppen `kod", "öppen `kod"),
        ("[text] utan mål och [](tom)", "[text] utan mål och [](tom)"),
        (
            r"\*inte betonad\* och \# tecken",
            "*inte betonad* och # tecken",
        ),
        ("åäö **ÅÄÖ** é", "åäö ÅÄÖ é"),
    ] {
        let doc = parse(src);
        assert_eq!(doc.blocks[0].text, plain, "{src}");
        if !src.contains("ÅÄÖ") {
            assert!(doc.blocks[0].runs.iter().all(|r| r.style == 0), "{src}");
        }
    }
    // A heading and a list item have inline markup too; code blocks none.
    let doc = parse("## En *rubrik*\n- med [länk](vy://a/b)\n```\n**inte fet**\n```");
    assert_eq!(doc.blocks[0].text, "En rubrik");
    assert_eq!(doc.blocks[1].runs.last().unwrap().link, Some(0));
    assert_eq!(doc.blocks[2].text, "**inte fet**");
    assert_eq!(count("två ord\n\noch  tre till"), (5, 22));
    assert_eq!(count(""), (0, 0));
}

// ---- the page ---------------------------------------------------------

fn page(width: f32, height: f32, storage: bool) -> Page {
    mock::reset();
    mock::with(|h| {
        h.caps.ime = true;
        h.caps.storage = storage;
    });
    init(width, height)
}

fn init(width: f32, height: f32) -> Page {
    Page::init(
        Window {
            width,
            height,
            scale: 1.0,
        },
        0,
    )
    .unwrap()
}

fn focus(p: &mut Page, node: i32) {
    if let Some(old) = p.focus {
        p.event(Event::Focus {
            node: NodeId(old),
            gained: false,
            keyboard: true,
        });
    }
    p.event(Event::Focus {
        node: NodeId(node),
        gained: true,
        keyboard: true,
    });
}

fn chord(p: &mut Page, code: i32, modifiers: i32) -> bool {
    p.event(Event::KeyDown {
        code,
        modifiers: Modifiers(modifiers),
        repeat: false,
    })
}

fn set_text(p: &mut Page, text: &str) {
    focus(p, ID_EDITOR);
    chord(p, key::A, 2);
    p.event(Event::TextInput { text });
}

/// The style id each drawn text of the last frame has.
fn drawn() -> Vec<(String, i32, f32, f32)> {
    mock::with(|h| {
        h.frame_ops()
            .iter()
            .filter_map(|op| match op {
                mock::Op::Text {
                    text,
                    style_id,
                    x,
                    y,
                    ..
                } => Some((text.clone(), *style_id, *x, *y)),
                _ => None,
            })
            .collect()
    })
}

/// SPEC-5.2.5: the preview is laid out by the page, a fragment for each
/// stretch of one style on one line, wrapped at the pane's width.
#[test]
fn the_preview_wraps_styled_text() {
    let mut p = page(1280.0, 800.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    let l = p.layout();
    assert!(l.show_editor && l.show_preview && !l.narrow);
    let style = |i: usize| p.types[i].id.0;
    let all = drawn();
    assert_eq!(all[0].1, p.title.0, "the page's own heading");
    // The preview's texts: those right of the editor.
    let ops: Vec<_> = all.into_iter().filter(|o| o.2 >= l.view.x).collect();
    let find = |text: &str| ops.iter().find(|o| o.0 == text).cloned();
    assert_eq!(find("Anteckningar").unwrap().1, style(S_H1));
    assert_eq!(find("Markdown").unwrap().1, style(S_BOLD));
    assert_eq!(find("sparas").unwrap().1, style(S_EMPH));
    assert_eq!(find("###").unwrap().1, style(S_CODE));
    assert_eq!(find("Vy på GitHub").unwrap().1, style(S_LINK));
    assert_eq!(find("fn main() {").unwrap().1, style(S_CODE));
    assert_eq!(find("Ett kodblock").unwrap().1, style(S_H1 + 2));
    assert_eq!(find("1.").unwrap().1, style(S_MARK));
    // Words of one style on one line are one fragment.
    assert!(find("Skriv ").is_none());
    assert!(ops.iter().any(|o| o.0 == "Skriv"));
    assert!(ops.iter().any(|o| o.0.starts_with("till vänster och läs")));
    // Nothing reaches past the text width, and lines go downwards.
    let width = l.text_w();
    for f in &p.preview.frags {
        assert!(f.x + f.w <= width + 0.01, "{f:?}");
    }
    for at in p.preview.placed.windows(2) {
        assert!(at[1].y >= at[0].y + at[0].h, "{at:?}");
    }
    let wide = p.preview.height;
    let frags = p.preview.frags.len();

    // The same text in a narrower pane wraps into more lines.
    p.resize(Window {
        width: 760.0,
        height: 800.0,
        scale: 1.0,
    });
    mock::run_frame(&mut p, 0).unwrap();
    assert!(p.preview.height > wide && p.preview.frags.len() > frags);
    let width = p.layout().text_w();
    assert!(p.preview.frags.iter().all(|f| f.x + f.w <= width + 0.01));
    // A frame that changes nothing keeps the layout.
    let laid = p.laid;
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(p.laid, laid);
    mock::with(|h| assert!(h.refusals.is_empty(), "{:?}", h.refusals));
}

/// SPEC-10.1.1: every block is a node with its role, list items inside a
/// `list`, the rule a `separator`, links `link` nodes with fixed ids.
#[test]
fn the_tree_has_every_block_with_its_role() {
    let mut p = page(1280.0, 800.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let count = |r: Role| h.sem.iter().filter(|n| n.role == r as i32).count();
        assert_eq!(count(Role::Heading), 4, "the page's and the note's three");
        assert_eq!(count(Role::List), 2);
        assert_eq!(count(Role::ListItem), 7);
        assert_eq!(count(Role::Separator), 1);
        assert_eq!(count(Role::Textbox), 1);
        // The editor's name is the `text` node right before it.
        let editor = h.sem.iter().position(|n| n.id == ID_EDITOR).unwrap();
        assert_eq!(
            (h.sem[editor - 1].role, h.sem[editor - 1].text.as_str()),
            (Role::Text as i32, "Anteckningens text i Markdown")
        );
        assert_eq!(count(Role::Link), 2);
        let list = h.sem.iter().find(|n| n.role == Role::List as i32).unwrap();
        let item = h
            .sem
            .iter()
            .find(|n| n.text == "punkter i två nivåer")
            .unwrap();
        assert_eq!(item.parent_id, list.id);
        let ids: Vec<i32> = h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect();
        assert_eq!(ids, [ID_EDITOR, ID_LINK, ID_LINK - 1], "Tab order");
        let link = h.sem.iter().find(|n| n.id == ID_LINK).unwrap();
        assert_eq!(link.text, "Vy på GitHub");
        assert!(h
            .sem
            .iter()
            .any(|n| n.text == "fn main() {\n    println!(\"Hej från Vy\");\n}"));
        assert!(h.sem.iter().any(|n| n.text == "98 ord · 630 tecken"));
    });
}

/// SPEC-5.2.10: links are registered with the host every frame and
/// Enter on a focused link navigates.
#[test]
fn spec_5_2_10_links_are_registered_and_followed() {
    let mut p = page(1280.0, 800.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let urls: Vec<&str> = h.links.iter().map(|l| l.5.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://github.com/hammandevab/vy",
                "vy://localhost:8090/hello"
            ]
        );
        assert!(h.links.iter().all(|l| l.3 > 0.0 && l.4 > 0.0));
    });
    focus(&mut p, ID_LINK - 1);
    assert!(chord(&mut p, key::ENTER, 0));
    mock::with(|h| {
        assert_eq!(
            h.navigations,
            [(String::from("vy://localhost:8090/hello"), false)]
        )
    });
    // The ring is drawn where focus is, and the frame still registers.
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| assert_eq!(h.links.len(), 2));

    // A link wrapped over two lines is two rectangles and one node; a
    // target that cannot be followed is drawn but not registered.
    set_text(
        &mut p,
        "Början [en mycket lång länktext som inte får plats på en rad i den här smala rutan alls](https://a.example) och [post](mailto:x@example.se).",
    );
    p.resize(Window {
        width: 760.0,
        height: 800.0,
        scale: 1.0,
    });
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        assert!(h.links.len() >= 2, "{:?}", h.links);
        assert!(h.links.iter().all(|l| l.5 == "https://a.example"));
        let ids: Vec<i32> = h.links.iter().map(|l| l.0).collect();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique, "one id a rectangle");
        let nodes = h.sem.iter().filter(|n| n.role == Role::Link as i32).count();
        assert_eq!(nodes, 2);
    });
    focus(&mut p, ID_LINK - 1);
    chord(&mut p, key::ENTER, 0);
    assert!(p
        .status_text()
        .ends_with("Länken kunde inte öppnas (E_ARG)."));
}

/// Typing changes the preview, the counts and the stored text; a new
/// instance starts with it (SPEC-5.2.9).
#[test]
fn typing_updates_the_preview_the_counts_and_the_storage() {
    let mut p = page(1280.0, 800.0, true);
    assert_eq!(p.words, (98, 630));
    set_text(&mut p, "# Ny\n\nEtt *nytt* stycke.");
    assert_eq!(p.words, (5, 24));
    assert_eq!(p.status_text(), "5 ord · 24 tecken");
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(
        p.preview
            .doc
            .blocks
            .iter()
            .map(|b| b.kind)
            .collect::<Vec<_>>(),
        [Kind::Heading(1), Kind::Para]
    );
    mock::with(|h| {
        assert_eq!(
            h.state.get(STORE.as_bytes()).map(Vec::as_slice),
            Some("# Ny\n\nEtt *nytt* stycke.".as_bytes())
        );
        assert!(h.ime_rect.is_some(), "the editor has composition open");
    });
    let again = init(1280.0, 800.0);
    assert_eq!(again.editor.text(), "# Ny\n\nEtt *nytt* stycke.");
    // An emptied note stays empty: the sample is for a first visit only.
    set_text(&mut p, "x");
    chord(&mut p, key::BACKSPACE, 0);
    let again = init(1280.0, 800.0);
    assert_eq!(again.editor.text(), "");
    assert_eq!(again.status_text(), "0 ord · 0 tecken");

    // The editor takes at most 6 000 bytes and a long text still draws.
    let long = "ord ".repeat(3000);
    set_text(&mut p, &long);
    assert!(p.editor.text().len() <= MAX_TEXT);
    assert!(mock::run_frame(&mut p, 0).unwrap() < 3000);
    mock::with(|h| assert!(h.refusals.is_empty(), "{:?}", h.refusals));

    let mut p = page(1280.0, 800.0, false);
    assert!(p.editor.text().starts_with("# Anteckningar"));
    assert!(p.status_text().ends_with("tecken · sparas inte"));
    set_text(&mut p, "ändå");
    assert_eq!(p.editor.text(), "ändå");
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        assert!(h
            .sem
            .iter()
            .any(|n| n.text == "1 ord · 4 tecken · sparas inte"))
    });
}

/// The preview scrolls to the block the caret is in.
#[test]
fn the_preview_follows_the_caret() {
    let mut p = page(1000.0, 420.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(p.scroll.offset(), 0.0);
    assert!(
        p.content_h() > p.layout().view.h,
        "the note is taller than the pane"
    );
    focus(&mut p, ID_EDITOR);
    chord(&mut p, key::END, 2);
    mock::run_frame(&mut p, 0).unwrap();
    let l = p.layout();
    assert_eq!(
        p.scroll.offset(),
        ScrollView::max_offset(l.view, p.content_h())
    );
    chord(&mut p, key::HOME, 2);
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(p.scroll.offset(), 0.0);
    // The wheel moves the preview on its own, and it stays where it was
    // put while the caret rests.
    p.event(Event::PointerMove {
        x: l.view.x + 50.0,
        y: l.view.y + 50.0,
        buttons: 0,
        modifiers: Modifiers(0),
    });
    assert!(p.event(Event::Scroll {
        dx: 0.0,
        dy: 120.0,
        lines: false,
        modifiers: Modifiers(0),
    }));
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(p.scroll.offset(), 120.0);
    // The keyboard's focus on a link below the pane brings it into view.
    focus(&mut p, ID_LINK - 1);
    mock::run_frame(&mut p, 0).unwrap();
    let r = p.link_rect(&p.layout(), 1).unwrap();
    assert!(r.y >= l.view.y && r.bottom() <= l.view.bottom(), "{r:?}");
}

/// Below 720 px the page shows one pane behind the tabs "Skriv" and
/// "Läs"; the pane that is not shown is not in the tree.
#[test]
fn a_narrow_window_has_tabs() {
    let mut p = page(400.0, 700.0, true);
    let l = p.layout();
    assert!(l.narrow && l.show_editor && !l.show_preview);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let roles = |r: Role| h.sem.iter().filter(|n| n.role == r as i32).count();
        assert_eq!(roles(Role::Tab), 2);
        assert_eq!(roles(Role::Textbox), 1);
        assert_eq!(roles(Role::Link), 0);
        assert!(h.sem.iter().any(|n| n.text == "Skriv (vald)"));
        assert!(h.links.is_empty());
    });
    let tab = l.tabs[1];
    assert!(p.event(Event::PointerDown {
        x: tab.x + 10.0,
        y: tab.y + 10.0,
        button: 1,
        modifiers: Modifiers(0),
    }));
    assert_eq!(p.pane, 1);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let roles = |r: Role| h.sem.iter().filter(|n| n.role == r as i32).count();
        assert_eq!(roles(Role::Textbox), 0);
        assert_eq!(roles(Role::Link), 2);
        assert!(h.sem.iter().any(|n| n.text == "Läs (vald)"));
        assert_eq!(h.links.len(), 2);
    });
    let width = p.layout().text_w();
    assert!(p.preview.frags.iter().all(|f| f.x + f.w <= width + 0.01));
    // With the keyboard: focus the other tab, Enter.
    focus(&mut p, ID_TAB);
    assert!(chord(&mut p, key::ENTER, 0));
    assert_eq!(p.pane, 0);
}

// ---- hostile texts ----------------------------------------------------

/// `md::link_at` as it was: a search to the end of the text from every
/// `[`. What a link is has not changed; this is its definition.
fn link_at_searching(b: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    let close = i + 1 + b[i + 1..].iter().position(|c| *c == b']')?;
    if b.get(close + 1) != Some(&b'(') {
        return None;
    }
    let end = close + 2 + b[close + 2..].iter().position(|c| *c == b')')?;
    (close > i + 1 && end > close + 2).then_some((close, close + 2, end))
}

/// The links of `src`, found as `inline` walks it: from each `[` in
/// turn, going on after a link's `)`.
fn links_of(
    src: &str,
    mut find: impl FnMut(&[u8], usize) -> Option<(usize, usize, usize)>,
) -> Vec<(usize, usize, usize, usize)> {
    let b = src.as_bytes();
    let (mut found, mut i) = (Vec::new(), 0);
    while i < b.len() {
        match (b[i] == b'[').then(|| find(b, i)).flatten() {
            Some((text_end, url, url_end)) => {
                found.push((i, text_end, url, url_end));
                i = url_end + 1;
            }
            None => i += 1,
        }
    }
    found
}

/// xorshift64*: the same texts in every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// SPEC-5.3.3 (granskning 0013:1): the parser looks at each byte of a
/// text a fixed number of times, whatever the text. A `[` used to start a
/// search for `]` to the end of the text, and a `](` one for `)`: 6 000
/// openers without a closer were 18 million steps and stopped the page on
/// its own saved note. The searches now remember what they found, so the
/// same texts take one pass each, and they find exactly the links they
/// found before.
#[test]
fn spec_5_3_3_hostile_markdown_is_searched_once() {
    let searched = |src: &str| {
        md::SEARCHED.with(|n| n.set(0));
        let doc = parse(src);
        (md::SEARCHED.with(core::cell::Cell::get), doc)
    };
    let cases = [
        "[".repeat(6000),
        "[".repeat(5999) + "]",
        "[a](".repeat(1500),
        "[".repeat(2000) + &"](".repeat(1000) + &"a".repeat(2000),
        "[a]".repeat(2000),
        "[a](b)".repeat(1000),
        "[[a](b)".repeat(857),
        "[]()".repeat(1500),
        "](".repeat(3000),
    ];
    for src in &cases {
        let (steps, doc) = searched(src);
        // One paragraph: a pass for `]` and a pass for `)` at most.
        assert!(
            steps <= 2 * src.len(),
            "{steps} bytes searched for {} bytes starting {:?}",
            src.len(),
            &src[..12]
        );
        assert_eq!(doc.blocks.len(), 1);
    }
    let (_, doc) = searched(&cases[5]);
    assert_eq!(doc.links.len(), 1000);
    assert_eq!(doc.blocks[0].text, "a".repeat(1000));
    let (_, doc) = searched(&cases[0]);
    assert_eq!(
        doc.blocks[0].text, cases[0],
        "openers without a closer are text"
    );
    assert!(doc.links.is_empty());

    // The same links as a search from every `[` finds, in texts made of
    // nothing but the characters that matter.
    let alphabet = ["[", "]", "(", ")", "](", "[a]", "a", " ", "\\", "`", "*"];
    let mut seed = 0x5EED_5EED_0000_0001u64;
    for _ in 0..400 {
        let mut src = String::new();
        for _ in 0..(next(&mut seed) % 60) {
            src.push_str(alphabet[(next(&mut seed) % alphabet.len() as u64) as usize]);
        }
        let (mut close, mut paren) = (md::Next::new(b']'), md::Next::new(b')'));
        let remembered = links_of(&src, |b, i| md::link_at(b, i, &mut close, &mut paren));
        assert_eq!(remembered, links_of(&src, link_at_searching), "{src:?}");
    }

    // Every other construct is linear already: an opener looks no
    // further than the next marker of its kind. A text of each, at the
    // editor's limit, parses to one block or one block a line.
    for unit in [
        "`", "`a", "*", "*a", "**", "**a", "_", "_a", "a_", "\\", "\\[", "***",
    ] {
        let src = unit.repeat(6000 / unit.len());
        assert_eq!(parse(&src).blocks.len(), 1, "{unit:?}");
    }
    assert_eq!(parse(&"a\n\n".repeat(2000)).blocks.len(), 2000);
    assert_eq!(parse(&"- a\n".repeat(1500)).blocks.len(), 1500);
    assert!(parse(&"\n".repeat(6000)).blocks.is_empty());
}

/// SPEC-5.3.3 (granskning 0013:1): a long note's three heavy steps each
/// get a frame of their own. In the frame where the editor wraps the text
/// the preview keeps its last layout and asks for another frame; the next
/// one parses, the one after lays out. A width change lays the same text
/// out again without a parse, a short note does everything in one frame,
/// and the preview a long note ends up with is the one a single pass
/// gives.
#[test]
fn spec_5_3_3_a_long_note_is_prepared_over_several_frames() {
    let mut p = page(1280.0, 800.0, true);
    let frames = || mock::with(|h| h.frame_requests);
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 0, "the sample is short: one frame");
    let sample_blocks = p.preview.doc.blocks.len();
    assert!(sample_blocks > 10);

    let long = "ett *två* [tre](vy://x)\n\n".repeat(200);
    assert!(long.len() > PHASED && long.len() <= MAX_TEXT);
    set_text(&mut p, &long);
    // 1: the editor wraps; the preview still shows the sample.
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 1);
    assert_eq!(p.preview.doc.blocks.len(), sample_blocks);
    assert!(p.parsed.is_none());
    // 2: the parse.
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 2);
    assert!(p
        .parsed
        .as_ref()
        .is_some_and(|(_, doc)| doc.blocks.len() == 200));
    assert_eq!(p.preview.doc.blocks.len(), sample_blocks);
    // 3: the layout, and nothing more is asked for.
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 2);
    assert_eq!(p.preview.doc.blocks.len(), 200);
    assert_eq!(p.preview.placed.len(), 200);
    assert_eq!(p.laid, Some((p.revision, p.layout().text_w())));
    mock::with(|h| {
        let links = h.sem.iter().filter(|n| n.role == Role::Link as i32).count();
        assert_eq!(links, 200, "the whole tree once the layout is ready");
        assert_eq!(h.links.len(), 200);
    });
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 2, "an unchanged note is drawn, nothing else");

    // The same preview as one pass over the same text gives.
    let phased = (p.preview.placed.clone(), p.preview.frags.clone());
    let width = p.layout().text_w();
    p.lay_out(parse(&long), width).unwrap();
    assert_eq!((p.preview.placed.clone(), p.preview.frags.clone()), phased);

    // A narrower window: the editor's frame, then the layout of the text
    // the preview already holds, with no parse between them.
    p.resize(Window {
        width: 1000.0,
        height: 800.0,
        scale: 1.0,
    });
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 3);
    assert_ne!(p.laid, Some((p.revision, p.layout().text_w())), "not yet");
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 3);
    assert!(p.parsed.is_none());
    assert_eq!(p.laid, Some((p.revision, p.layout().text_w())));
    assert_eq!(p.preview.doc.blocks.len(), 200);

    // Typing while the preview is behind starts it over for the new text.
    p.event(Event::TextInput { text: "x" });
    mock::run_frame(&mut p, 0).unwrap();
    p.event(Event::TextInput { text: "y" });
    mock::run_frame(&mut p, 0).unwrap();
    mock::run_frame(&mut p, 0).unwrap();
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(p.laid, Some((p.revision, p.layout().text_w())));
    assert!(p.preview.doc.blocks.last().is_some_and(|b| b.text == "xy"));

    // Behind the tabs of a narrow window the preview has the frame to
    // itself: parse, then layout.
    let mut p = page(400.0, 700.0, true);
    set_text(&mut p, &long);
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(frames(), 0, "the editor alone");
    let read = p.layout().tabs[1];
    p.event(Event::PointerDown {
        x: read.x + 4.0,
        y: read.y + 4.0,
        button: 1,
        modifiers: Modifiers(0),
    });
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!((frames(), p.preview.doc.blocks.len()), (1, 0));
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!((frames(), p.preview.doc.blocks.len()), (1, 200));
}
