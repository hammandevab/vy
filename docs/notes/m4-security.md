# M4 – säkerhet: förtroendekedjan, processgränsen och fuzzning

Byggagentens noteringar för grenen `m4/security` (från `main` 3bc0bcb). Granskningssidan skrivs
av en annan agent. Spåret äger `crates/vy-proto`, `crates/vy-manifest`, nyckel-/loggvägarna i
`crates/vy-serve`, `crates/vy-shell/src/process.rs` och `sandbox.rs`, `fuzz/`,
`.github/workflows/fuzz.yml` och gräns-/isoleringstesterna i `crates/vy-runtime/src/tests.rs`.
Filer utanför det står under *Berör andra spår*.

Spec-frågor numreras från 53 (BUILD-NOTES och notes/m3-shell slutar på 52); de parallella
M4-spåren kan ha valt samma nummer, så de kan behöva numreras om vid merge.

## Läge

| Steg | Innehåll | Läge |
| --- | --- | --- |
| 1 | DNS TXT `_vy.<värd>` (SPEC-7.4.2, ny 7.4.5) | klart |
| 2 | Rotation end-to-end, pensionering av den gamla nyckeln (SPEC-7.4.4, ny 7.4.6) | klart |
| 3 | Transparenslogg (ny §7.6) | – |
| 4 | OS-sandlåda för flikprocessen (SPEC-9.1.1, 9.1.2, 11.2.2, ny 9.1.3) | – |
| 5 | Resursgränser (SPEC-9.3.1) och isolering per origin (SPEC-9.5.1) | – |
| 6 | Fuzzning | – |

## Steg 1 – DNS TXT (SPEC-7.4.2, SPEC-7.4.5)

Arbetet började hos en tidigare agent som stoppades före första commit; allt var oincheckat och
oprovat. Det är genomläst, rättat (nedan) och incheckat i fyra commits:

| Commit | Innehåll |
| --- | --- |
| `spec:` SPEC-7.4.5 | Postens form, vad första besöket kräver, localhost-undantaget, pinnad origin frågar aldrig. `vy-manifest`: `dns_txt_name`, `dns_txt_record`, `parse_dns_txt`, `dns_txt_zone_line`; test `spec_7_4_5_dns_txt_record_form`. Rör inget fryst (§7 är protokollet, inte §5/bilaga A/C). |
| `vy-proto` DNS-klient | `dns.rs`: `TxtResolver` (trait), `SystemResolver`, `build_query`/`parse_answer` som rena funktioner. |
| `vy-proto` nyckelkontroll | `Fetcher::with_resolver`, `Restriction` och `Fetched::restriction()`; `spec_7_4_2_dns_txt_must_agree` un-ignorerad. |
| `vy-serve` | TXT-raden vid start och på `/.well-known/vy-dns`; `vy-publish keygen` skriver posten. |

**Postens form.** `v=vy1; k=<64 gemena hex>` vid `_vy.<värd>` – samma nyckeltext som en rad i
`/.well-known/vy-key`, en post per nyckel. Exakt den formen godtas (ett mellanslag efter `;`,
gemener, 64 tecken, en giltig kurvpunkt); andra TXT-poster på namnet (SPF m.fl.) är inte fel, de
är bara inte Vy:s. `v=vy1` gör att en senare form kan dela namnet.

**Semantik.** Vid första besöket av en origin (ingen pinnad nyckel) hämtas nyckelfilen och
TXT-posterna; bara nycklar som står i *båda* pinnas, och sidan är betrodd bara om den är signerad
med en av dem. Saknad post, ingen gemensam nyckel och ett uppslag som misslyckas (timeout,
SERVFAIL, socketfel, ogiltigt namn) ger begränsat läge, **ingen pin**, och nästa besök frågar
igen – stängt vid fel. En origin med pinnad nyckel frågar aldrig DNS igen; ny nyckel bara genom
rotation. `localhost`, `127.0.0.1` och `[::1]` frågar inte (som TLS-undantaget i SPEC-7.1.3), så
den lokala utvecklingsloopen med `vy-serve` fungerar som förr. Andra IP-literaler frågas och
faller (det finns inget `_vy.<ip>`): stängt.

**Orsaken.** `Fetched::restriction()` ger `Restriction` med `Display`: `Unsigned`,
`BadSignature`, `UnknownKey`, `KeyFileUnavailable`, `KeyNotPublished`, `DnsNoRecord`,
`DnsMismatch`, `DnsFailed`, `SuccessorNotPublished`. Skalet visar i dag bara markören
"begränsat läge"; att visa texten bredvid är m4/ux:s (se *Berör andra spår*).

**DNS-klienten.** Ingen resolvercrate (hickory drar in tokio och ett tjugotal crates).
Unix: frågan byggs för hand (RFC 1035, RD satt, ett namn, typ TXT, klass IN) och skickas över
UDP till upp till tre `nameserver` ur `/etc/resolv.conf` (127.0.0.1 om filen saknar rader, som
libc), uppkopplad socket (bara serverns datagram tas emot), slumpat id ur `getrandom`, OS-valt
källport; svaret måste ha QR, rätt id, exakt vår fråga (namn, typ, klass – skiftlägesokänsligt),
annars väntar klienten vidare till tidsgränsen. TC ger samma fråga över TCP. RCODE 3 (NXDOMAIN)
och tomt svar är "inga poster"; övriga RCODE är fel. Bara svarsavsnittets TXT-poster för det
frågade namnet, eller för ett namn som en CNAME i samma svar pekar ut (högst åtta), läses; ett
TXT för något annat namn i svaret tas inte. Namnkomprimering följs bara bakåt, och namnlängden är
begränsad, så en pekarslinga avslutas. 2 s per försök, tre försök över servrarna i tur och
ordning: ett uppslag som misslyckas tar högst 6 s, en gång per origin och besök tills det lyckas.
Windows: `DnsQuery_UTF8` med `DNS_TYPE_TEXT`, namnet med avslutande punkt (inget söksuffix),
bara poster i svarsavsnittet; systemets resolver sköter tidsgränser, försök och CNAME.
Söksuffix (`search`/`ndots`) används aldrig: `_vy.<värd>` frågas som det står.

**Rättat i den tidigare agentens arbete.** (a) SPEC-7.4.4 hade fått en tilläggsmening om
pensionering mitt i steg 1; den är återställd och beslutet blir en egen mening i steg 2.
(b) SPEC-7.4.5 sade "orsaken synlig för användaren", vilket inget test här kan bevisa (skalets
UI är m4/ux:s); nu "med orsaken angiven så att klienten kan visa den". (c) Svarstolken tog TXT-
poster för vilket ägarnamn som helst i svaret; nu bara det frågade namnet och dess CNAME-kedja.
(d) Windows läste poster ur alla avsnitt och utan avslutande punkt; nu svarsavsnittet och FQDN.
(e) `vy-serve` skrev `_vy.localhost.`-rader vid start; nu "inget behövs för loopback", och en
varning för `*.up.railway.app`. (f) Felmeddelandet för `KeyNotPublished` nämnde DNS även på
loopback.

### Hotbild – DNS TXT

- **Vad.** Ett nytt nätverksindata: DNS-svar från systemets resolver, tolkade i klienten vid
  första besöket av varje fjärr-origin; en ny utgång: frågan `_vy.<värd>` (avslöjar värden för
  resolvern, vilket A-uppslaget redan gör).
- **Vem kan missbruka.** (1) En angripare på vägen till resolvern som förfalskar UDP-svar
  (Kaminsky-typ): för att få en egen nyckel pinnad måste hen *också* styra
  `/.well-known/vy-key` över TLS – DNS ensam ger aldrig förtroende, bara färre. (2) En fientlig
  eller trasig resolver: kan bara göra sidor begränsade (DoS på capabilities), aldrig betrodda.
  (3) Ett fientligt svar mot tolken: minnesfel, slingor, stora allokeringar.
- **Hur begränsas det.** Snittet av två oberoende källor; stängt vid varje fel; ingen pin vid
  fel; uppkopplad UDP-socket med slumpat 16-bitars id och OS-slumpad källport; svaret måste
  upprepa frågan; tolken är säker Rust (`unsafe` bara i Windows-anropet), läser högst
  4 096 byte UDP / 65 535 byte TCP, högst 64 poster, bara bakåtpekare, namn ≤ 255 byte.
  Pinnade origins frågar aldrig, så en senare DNS-kapning påverkar inte en origin man redan
  besökt.
- **Test som bevisar begränsningen.** `spec_7_4_2_dns_txt_must_agree` (stämmer, avviker, saknas,
  timeout, nyckel bara i DNS, pinnad utan uppslag, loopback utan uppslag);
  `dns::tests::spec_7_4_2_txt_wire_format` (fel id, fel fråga, inte ett svar, trunkerad rdata,
  framåtpekare, självpekare, 64-bytes etikett, CNAME-kedja, främmande ägarnamn);
  `dns::tests::system_resolver_udp_then_tcp` (datagram med fel id ignoreras, TC → TCP, tyst
  server → timeout); fuzzmålet för svarstolken (steg 6).

### Railway-sajten visas i begränsat läge vid första besök

`vy-production-d83b.up.railway.app` är en underdomän till `railway.app`; ägaren kan inte lägga
in TXT-poster där. **Från och med den här grenen ser en klient som besöker sajten för första
gången varje sida i begränsat läge**: sidorna ritas, men inga capabilities beviljas (chat får
ingen `net`-fråga och är offline, settings sparar inget, form får ingen `ime`), och markören
"begränsat läge" syns. Orsaken är `DnsNoRecord { name: "_vy.vy-production-d83b.up.railway.app" }`.
En klient som redan pinnat nyckeln (testare som körde testomgång 1) påverkas inte – pinnade
origins frågar aldrig DNS. Testomgång 1:s steg 2 ("ingen röd markering") stämmer alltså inte för
en ny testare förrän något av nedanstående är gjort.

Förslag, i ordning:

1. **Egen domän på Railway** (rekommenderas). I Railway: tjänsten → Settings → Networking →
   Custom Domain, t.ex. `vy.<ägarens-domän>.se`; lägg in den CNAME Railway anger hos
   DNS-leverantören. Sätt `VY_HOST_NAME=vy.<ägarens-domän>.se` i tjänstens variabler. Starta om:
   `vy-serve` skriver då raden att publicera, t.ex.
   `_vy.vy.<ägarens-domän>.se. IN TXT "v=vy1; k=<nyckel>"` (även på
   `https://vy.<ägarens-domän>.se/.well-known/vy-dns`). Lägg in TXT-posten hos samma leverantör.
   CNAME:n på `vy.…` och TXT:n på `_vy.vy.…` är olika namn och krockar inte. Kontroll:
   `dig +short TXT _vy.vy.<ägarens-domän>.se`. Byggsecret `vy_key` måste vara satt så att
   nyckeln är stabil mellan byggen (notes/m3-proto-serve), annars stämmer posten bara till nästa
   bygge.
2. **Ingen genväg i klienten.** Ett undantag för `*.railway.app` eller en miljövariabel som
   stänger av kontrollen vore ett hål för varje sajt på samma plattform; specen säger stängt.
3. Tills 1 är gjort: säg till testare att begränsat läge på Railway-adressen är väntat, eller
   låt dem köra `vy-serve` lokalt (`vy://localhost:8080/…` är undantaget).

## Steg 2 – rotation end to end (SPEC-7.4.4, SPEC-7.4.6)

`spec_7_4_4_key_rotation` (vy-proto, mot testservern) går hela kedjan: ett manifest signerat med
den gamla nyckeln bär `next_key_id` → efterträdaren antecknas → ett manifest signerat med den nya
nyckeln får klienten att hämta `/.well-known/vy-key` → den nya nyckeln godtas och pinnas.

**Beslut: den gamla nyckeln pensioneras** (ny SPEC-7.4.6). I samma ögonblick som ett manifest
signerat med efterträdaren verifierat tas *alla* nycklar som var pinnade för origin bort; sidor
som fortfarande är signerade med en av dem körs i begränsat läge (`UnknownKey`) även om
nyckelfilen fortfarande listar dem. Motivering: rotation används för att ta en nyckel ur bruk
(läcka, byte av utgivare); en klient som fortsatte godta den gamla gav en stulen nyckel evigt
liv, och "ingen annan väg ger en okänd nyckel förtroende" (7.4.4) vore halvt uppfyllt. Följd
för utgivaren: signera om alla sidor med den nya nyckeln i samma publicering (`vy-publish
build` gör det per sida). Övergångsfönster: mellan annonseringen och första manifestet med den
nya nyckeln gäller den gamla som förr. HTTP-cachen kan under `max-age` (60 s från vy-serve)
ge ett manifest signerat med den gamla nyckeln för en annan sökväg; det körs begränsat tills det
revalideras. Alternativen som valdes bort: (a) gamla nyckeln gäller för alltid (M3:s beteende),
(b) pensionering redan vid annonseringen – då blir alla ännu inte omsignerade sidor begränsade
direkt, och annonsen är ju signerad med den gamla.

Övriga fall i testet: en annonserad nyckel som ännu inte står i nyckelfilen ger
`SuccessorNotPublished` och annonsen ligger kvar; ett manifest som namnger efterträdarens id men
är signerat av någon annan ger `BadSignature` och ändrar ingenting i förrådet (signaturen prövas
*innan* `promote`); en nyckel som publiceras utan kedja från ett betrott manifest ger
`UnknownKey`; nyckelfilen hämtas bara vid första besök och för en annonserad efterträdare (fyra
gånger i testet).

### Hotbild – rotation

- **Vad.** Nyckelförrådets enda väg till en ny nyckel för en pinnad origin, och nu även den väg
  som tar bort nycklar.
- **Vem kan missbruka.** (1) Den som stulit den gamla nyckeln men inte styr servern: kan annonsera
  sin egen nyckel, men den hämtas bara från `/.well-known/vy-key` över TLS – utan servern blir
  ingen rotation av. (2) Den som stulit nyckeln *och* styr servern: kan rotera till sin egen nyckel
  och pensionera den riktiga för klienter som ser rotationen; det är full kompromettering i
  vilket fall som helst, och transparensloggen (steg 3) gör rotationen synlig i efterhand. (3) En
  sida som försöker få klienten att pensionera nycklar med ett förfalskat manifest.
- **Hur begränsas det.** Efterträdaren måste vara annonserad av ett manifest som verifierat under
  en pinnad nyckel, stå i nyckelfilen och ha signerat det manifest som utlöser rotationen;
  förrådet ändras först efter verifieringen. `next_key_id` måste vara 16 hextecken (granskning
  0003 fynd 1).
- **Test som bevisar begränsningen.** `spec_7_4_4_key_rotation`,
  `spec_7_4_4_next_key_id_cannot_forge_pins`, `keys::tests::pins_and_successors_persist`.

## Byggmiljön (för den som tar över)

- `target/` är delad mellan fyra arbetsträd. Cargos metadatahash för en sökvägscrate räknas
  relativt arbetsytans rot, så samma crate i två arbetsträd fick samma artefakt och ett träd
  kunde länka ett annat träds bygge (observerat: vy-proto länkade en vy-manifest utan de nya
  funktionerna). Koordinatorn har sedan 19:31 lagt en ospårad `.cargo/config.toml` per
  arbetsträd med unika profilvärden per arbetsytecrate (`strip = "debuginfo"`,
  `codegen-units`), så hasharna skiljer sig. Checka inte in den filen (den står i
  `.git/info/exclude`).
- `spec_5_3_3_bench_32_rounded_clips_2000_rects` i `vy-render` (1 s-tak i debug) faller när
  maskinen är lastad (lastmedel ~5 på 2 CPU:er med fyra agenter) och går igenom ensam
  (`--test-threads=1`). Inte rört här; m4/perf äger testet.
- Windows/macOS typkontrolleras med `cargo check --target x86_64-pc-windows-msvc` /
  `aarch64-apple-darwin` och attrapper för C-verktygen (`CC_x86_64_pc_windows_msvc=
  /root/.local/bin/fake-msvc-cc`, `AR_…=fake-msvc-lib`, för macOS `fake-unix-ar`) – ring och
  zstd-sys kör cc i sina byggskript; inget länkas.

## Nya beroenden

Inga nya crates. `vy-proto` fick två direkta beroenden som redan fanns i trädet med samma
version: `getrandom 0.2` (fanns via ring och vy-shell) för DNS-id, och på Windows
`windows-sys 0.61` (fanns via vy-shell/softbuffer/winit) med två funktionsgrindar till,
`Win32_Foundation` och `Win32_NetworkManagement_Dns`. `cargo tree -p vy-proto -e normal`: 71
unika rader på Linux, 72 för Windows-målet; inget nytt i `Cargo.lock` utom de två kanterna.

## Berör andra spår

- `crates/vy-publish/src/main.rs` (m4/dev, m4/perf): en `println!` i `cmd_keygen` som skriver
  TXT-posten. Inget annat i vy-publish.
- `crates/vy-serve/src/lib.rs` (m4/dev lägger till funktioner): ny route
  `/.well-known/vy-dns`, `Site::dns_zone_lines`, `pub fn is_loopback_host`, fältet `keys` i
  `Site` och en doc-rad på `Stats::keys`. Inga publika namn ändrade eller borttagna.
- **m4/ux (vy-shell):** `vy_proto::Fetched::restriction()` finns nu; SPEC-7.4.5 vill att orsaken
  kan visas – t.ex. som verktygstips eller rad under markören "begränsat läge". `page.rs`
  kopierar i dag bara `trusted`.

## Spec-frågor

53. **SPEC-7.4.5 – fler än en nyckel.** Första besöket pinnar alla nycklar som står i båda
    källorna, inte bara den som signerat sidan (som M3 gjorde med nyckelfilen). En utgivare
    med två aktiva nycklar behöver alltså två TXT-poster. Avsett?
54. **SPEC-7.4.5 – väntetid.** Ett uppslag som inte svarar tar upp till 6 s och sker i skalets
    laddartråd (som är seriell, granskning 0004 fynd 7). Bör specen ge en tidsgräns, och bör
    sidan visas i begränsat läge medan uppslaget pågår?
55. **SPEC-7.4.5 – IP-literaler.** `vy://192.0.2.1/` frågar `_vy.192.0.2.1` och faller till
    begränsat läge. Det finns ingen DNS-zon för en adress; ska IP-adresser (utom loopback) vara
    otillåtna i `vy://`, eller alltid begränsade?
