//! The report's text, table and links. Fixed data so the page is the same
//! on every host (SPEC-2.1.5).

/// A block in the body flow: a subheading or a paragraph.
pub enum Block {
    Heading(&'static str),
    Para(&'static str),
}

pub const TITLE: &str = "Lägesrapport: Vy-klienten efter M1";
pub const AUTHOR: &str = "Maja Ek";
pub const AUTHOR_URL: &str = "vy://docs.example/personer/maja-ek";
pub const BYLINE_TAIL: &str = " · 4 november 2026 · 6 min läsning";
pub const NAV_LABEL: &str = "Innehåll:";
pub const LEAD: &str = "Den första milstolpen är nådd: tjugo guldbilder är gröna på Linux och macOS, och klienten renderar referenssidorna utan fönster. Den här rapporten sammanfattar vad som mättes, vad som fick ändras i specen och vad som återstår innan ABI 1.0 kan frysas.";

/// Section ids in `NAV` order; each points at the body block it starts at.
pub const SECTIONS: [(&str, &str, usize); 3] = [
    (
        "Bakgrund",
        "vy://docs.example/lagesrapport?avsnitt=bakgrund",
        0,
    ),
    ("Metod", "vy://docs.example/lagesrapport?avsnitt=metod", 3),
    (
        "Resultat",
        "vy://docs.example/lagesrapport?avsnitt=resultat",
        6,
    ),
];

pub const BODY: [Block; 9] = [
    Block::Heading("Bakgrund"),
    Block::Para("Vy är en klient för sidor som är WebAssembly-moduler. En sida ritar sig själv genom ett värd-API med färre än fyrtio funktioner, och allt som inte är ren beräkning eller ritning går genom en capability som manifestet begär. Klienten innehåller ingen tolk: det som körs är färdigkompilerad kod, och det som ritas är en display list som värden rasteriserar."),
    Block::Para("M1 skulle visa att den modellen håller i praktiken. Målet var en headless-väg från modul till PNG som ger samma bild byte för byte på två plattformar, så att konformanssviten kan vara det enda som avgör om en klient är en Vy-klient. Utan den vägen kan ingen referenssida granskas av en maskin, och principen om verifiering före läsbarhet skulle bli en önskan i stället för en grind."),
    Block::Heading("Metod"),
    Block::Para("Varje referensfall renderas tjugo gånger i följd med samma modul, samma fönster och samma scriptade händelser. Display listen jämförs först som bytes, därefter rasteriseras den och bilden jämförs pixel för pixel med guldbilden. Ett fall godkänns när högst en promille av pixlarna avviker, vilket i praktiken bara lämnar utrymme för avrundning i kantutjämningen."),
    Block::Para("Tiden mäts som logisk tid i bränsleenheter, inte som väggklocka. En miljon enheter motsvarar en millisekund på referensmaskinen, och värdens eget arbete i importerna debiteras samma budget med trettiotvå enheter per byte. Det gör måtten jämförbara mellan maskiner på bekostnad av att de är ungefärliga på var och en; tabell 1 anger därför både opkoder och logisk tid."),
    Block::Heading("Resultat"),
    Block::Para("Alla fjorton fall i första halvan av M2 ligger under två millisekunder för rasterisering och under en halv millisekund för modulens eget arbete. Textsidan med nittionio block är dyrast, eftersom varje synligt block formas på nytt i varje frame; en textcache i M3 bör ta bort det mesta av kostnaden. Bildsidorna avkodas en gång per instans och kostar därefter nästan ingenting."),
    Block::Para("Två avvikelser är värda att nämna. Fet serif saknas i de inbyggda typsnitten och matchas till sans, vilket syns i titlar som begär serif med vikt över sexhundra. Och guldbilderna är tagna på x86_64; macOS-jobbet på ARM rapporterar ännu noll avvikande pixlar, men det är en observation, inte ett bevis, och tröskeln lättas inte om den dagen kommer."),
];

pub const TABLE_CAPTION: &str = "Tabell 1. Rasterisering per referensfall, P95 över 100 körningar";
pub const TABLE_HEAD: [&str; 4] = ["Fall", "Opkoder", "Tid (ms)", "Cacheträff"];
/// The same columns, headed for a narrow window.
pub const TABLE_HEAD_NARROW: [&str; 4] = ["Fall", "Opkoder", "ms", "Cache"];
pub const TABLE_ROWS: [[&str; 4]; 12] = [
    ["hello-800x600@1", "4", "0,31", "–"],
    ["hello-800x600@2", "4", "0,74", "–"],
    ["text-800x600@1", "42", "1,84", "97 %"],
    ["text-800x600@1-scrolled", "40", "1,79", "97 %"],
    ["text-400x700@1", "38", "1,52", "96 %"],
    ["text-1024x768@1.5", "44", "2,61", "95 %"],
    ["form-800x600@1", "31", "0,92", "99 %"],
    ["form-800x600@1-filled", "35", "1,05", "98 %"],
    ["form-800x600@1-submitted", "36", "1,11", "98 %"],
    ["form-400x700@2", "33", "1,98", "97 %"],
    ["chat-800x600@1", "47", "1,40", "98 %"],
    ["shop-1280x800@1", "71", "1,93", "99 %"],
];
pub const CSV_LABEL: &str = "Ladda ner mätdata (CSV)";
pub const CSV_URL: &str = "https://docs.example/vy/m1-bench.csv";

pub const NOTES_HEADING: &str = "Noter";
/// Footnote text and the link printed under it.
pub const NOTES: [(&str, &str, &str); 3] = [
    (
        "1. Referensmaskinen är en bärbar dator från 2020 med integrerad grafik och 8 GB minne; P95 över 100 körningar enligt SPEC §13.",
        "Specifikation 1.0, avsnitt 13",
        "vy://docs.example/spec?avsnitt=13",
    ),
    (
        "2. Cacheträff avser textcachen i värden; strecket betyder att fallet inte formar text som kan återanvändas mellan frames.",
        "Mätprotokoll för M1",
        "vy://docs.example/protokoll/m1",
    ),
    (
        "3. macOS-jobbet kör samma svit på ARM; en avvikelse rapporteras som diffkvot i byggnoteringarna utan att tröskeln ändras.",
        "Byggnoteringar på GitHub",
        "https://github.com/hammandevab/vy/blob/main/docs/BUILD-NOTES.md",
    ),
];
