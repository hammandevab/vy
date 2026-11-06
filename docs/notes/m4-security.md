# M4 – säkerhet: förtroendekedjan, processgränsen och fuzzning

Byggagentens noteringar för grenen `m4/security` (från `main` 3bc0bcb). Granskningssidan skrivs
av en annan agent. Spåret äger `crates/vy-proto`, `crates/vy-manifest`, nyckel-/loggvägarna i
`crates/vy-serve`, `crates/vy-shell/src/process.rs` och `sandbox.rs`, `fuzz/`,
`.github/workflows/fuzz.yml` och gräns-/isoleringstesterna i `crates/vy-runtime/src/tests.rs`.
Filer utanför det står under *Berör andra spår*.

Spec-frågor numrerades från 53 (BUILD-NOTES och notes/m3-shell slutar på 52); vid
sammanslagningen i `m4/integration` blev de 58–62 i `docs/BUILD-NOTES.md` (M4), och numren nedan
är de nya.

## Läge

| Steg | Innehåll | Läge |
| --- | --- | --- |
| 1 | DNS TXT `_vy.<värd>` (SPEC-7.4.2, ny 7.4.5) | klart |
| 2 | Rotation end-to-end, pensionering av den gamla nyckeln (SPEC-7.4.4, ny 7.4.6) | klart |
| 3 | Transparenslogg (ny §7.6, SPEC-7.6.1–7.6.3) | klart |
| 4 | OS-sandlåda för flikprocessen (SPEC-9.1.1, 9.1.2, 11.2.2, ny 9.1.3) | klart (Linux provat, Windows/macOS typkontrollerat) |
| 5 | Resursgränser (SPEC-9.3.1) och isolering per origin (SPEC-9.5.1) | klart |
| 6 | Fuzzning | klart (cargo-fuzz-mål och nattjobb; lokalt bara den stabila mutationsvägen) |
| – | Flikprocesser från den körande avbilden (`self_exe`, koordinatorns uppdrag) | klart |

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
begränsad, så en pekarslinga avslutas. 2 s per utbyte, tre försök över servrarna i tur och
ordning, och en gemensam tidsgräns för hela uppslaget (`SystemResolver::deadline`, 6 s), UDP och
TCP inräknade: ett uppslag som misslyckas tar högst 6 s, en gång per origin och besök tills det
lyckas. Före granskning 0007 fynd 2 gällde TCP-vägens 2 s varje enskild läsning, så en resolver som
skickade en byte i taget kunde hålla uppslaget i timmar; TCP-svaret är nu också högst 16 384 byte
(`MAX_TCP_ANSWER`), och ett längre nekas på sitt längdprefix innan något läses.
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
  4 096 byte UDP / 16 384 byte TCP, högst 64 poster, bara bakåtpekare, namn ≤ 255 byte; hela
  uppslaget har en tidsgräns på 6 s.
  Pinnade origins frågar aldrig, så en senare DNS-kapning påverkar inte en origin man redan
  besökt.
- **Test som bevisar begränsningen.** `spec_7_4_2_dns_txt_must_agree` (stämmer, avviker, saknas,
  timeout, nyckel bara i DNS, pinnad utan uppslag, loopback utan uppslag);
  `dns::tests::spec_7_4_2_txt_wire_format` (fel id, fel fråga, inte ett svar, trunkerad rdata,
  framåtpekare, självpekare, 64-bytes etikett, CNAME-kedja, främmande ägarnamn);
  `dns::tests::system_resolver_udp_then_tcp` (datagram med fel id ignoreras, TC → TCP, tyst
  server → timeout); `dns::tests::system_resolver_has_one_deadline` (TCP-svar som droppar en byte
  per 1,5 s → timeout inom 6 s; längd 0xFFFF → nekad direkt); fuzzmålet för svarstolken (steg 6).

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

## Steg 3 – transparenslogg (§7.6, SPEC-7.6.1–7.6.3)

**Formatet (SPEC-7.6.1, `vy_manifest::{LogEntry, KeyLog}`).** En textfil, en post per rad: gemen
hex av den kanoniska CBOR-kodningen av `{host, key_id, key, published, prev_hash}` (nyckelordning
`key`, `host`, `key_id`, `prev_hash`, `published`). `host` är originens auktoritet som i
`vy://`-adressen (`exempel.se`, `localhost:8080`), `key_id` måste vara nyckelns härledda id,
`published` `ÅÅÅÅ-MM-DD`, `prev_hash` SHA-256 av föregående posts kodning (32 nollbyte för den
första). Huvudet är SHA-256 av sista posten. `#`-rader och tomma rader är kommentarer; högst 1 MB.
Varje rad avkodas strikt (kanonisk form, inga okända fält, host bara `[a-z0-9.-:[]]`, så en post
kan inte smuggla en rad till nyckelförrådet). Speglingsbar: vem som helst kan kopiera filen och
kontrollera kedjan; en logg kan bära poster för flera värdar (projektets egen logg i §15/§17 är
samma format).

**Utgivarens sida.** `vy-publish log-append --log <sajt>/vy-log --host <värd> --public-key
<dev.key.pub>` vid keygen och vid varje rotation (keygen skriver kommandot); en bruten logg
förlängs aldrig, en nyckel som redan står för värden läggs inte till igen, filen ersätts atomärt.
`vy-serve` serverar `<katalog>/vy-log` som den är på `/.well-known/vy-log` (läst per förfrågan,
`no-cache`) och skriver längd och huvud vid start, eller en varning om kedjan inte håller.

**Klientens sida (SPEC-7.6.2, 7.6.3).** Loggen hämtas bara när en nyckel avgörs: vid första besök
och vid rotation, aldrig för en pinnad nyckel. Serveras en logg pinnas bara nycklar som loggen
listar för originens host (utöver vy-key och DNS), och längd + huvud sparas bredvid pinnen
(`log <origin> <n> <hex>` i `keys`). Vid nästa nyckelbeslut måste loggen fortfarande ha det
sparade huvudet på samma plats (bara vuxit), annars `LogRewritten`. **Beslut: ingen logg (404) är
tillåten i 1.0** – annars vore varje befintlig sajt begränsad – men bara tills origin en gång
serverat en: därefter är en saknad logg `LogWithdrawn` (en spärrhake, så att en angripare inte
kan kringgå loggen genom att ta bort den). En logg som inte går att hämta eller tolka är
`LogInvalid`. Alla fall är begränsat läge utan att pinnarna ändras.

### Hotbild – transparensloggen

- **Vad.** Ny indata (loggfilen över TLS, vid nyckelbeslut), ny rad i nyckelförrådet, ny
  route i vy-serve, nytt kommando i vy-publish.
- **Vem kan missbruka.** (1) Den som stulit utgivarens nyckel och styr servern: kan rotera
  till en egen nyckel – men måste då lägga den i loggen, där den syns för speglar och
  utgivaren; att skriva om historiken upptäcks av varje klient som sett huvudet. (2) En server
  som visar olika loggar för olika klienter (split view): upptäcks bara genom speglar som
  jämför huvuden – utanför 1.0 (spec-fråga 61). (3) En fientlig logg mot tolken: storlek,
  kanonisk form, injektion i förrådet.
- **Hur begränsas det.** Kedjan kontrolleras rad för rad; huvudet sparas och måste finnas kvar;
  spärrhaken efter första loggen; 1 MB-tak före tolkning; strikt avkodning; host-tecken som
  inte kan bryta en rad; loggen ändrar aldrig pinnar på egen hand – den kan bara säga nej.
- **Test som bevisar begränsningen.** `spec_7_6_1_log_format_and_chain` (ändrad byte, tappad,
  bytt och omordnad post, annan kodning, okänt fält, förfalskat id, farlig host, för stor),
  `spec_7_6_2_7_6_3_key_log_checked_at_key_decisions` (saknad nyckel, trasig logg, annan host,
  listad, rotation utan loggpost, omskriven, borttagen, vuxen, ingen logg),
  `spec_7_6_1_log_served`, `spec_7_6_1_log_append`.

## Steg 4 – OS-sandlåda för flikprocessen (SPEC-9.1.3)

Vad flikprocessen gör efter uppstart, ur `strace -f` av `vy-shell --screenshot` med shop och
game: `read(0)`, `write(1)`, `mmap`/`mprotect`/`munmap`/`brk`, `memfd_create` + `write` +
`fcntl(F_ADD_SEALS)` (wasmtimes minnesavbilder), `sigaltstack`/`rt_sigaction` (wasmtimes
fällor), `getrandom`, `getpid`/`gettid`, `exit_group`. Typsnitten är inbyggda (`include_bytes!`
i vy-render), lagring och hämtning går över IPC: inget i fliken behöver en fil eller en socket.
Spelinspelning (`--record`/`--replay`) öppnar en fil i loopen och låses därför inte (bara
tester använder flaggorna). Låsningen sker i `--tab`-grenen i `vy-shell` och `vy-cli` innan
första meddelandet läses (`sandbox::lockdown`), så ingen sidkod kör med mer.

| Plattform | Standard | `VY_SANDBOX=strict` | Provat |
| --- | --- | --- | --- |
| Linux | fd > 2 stängda (`close_range`), `no_new_privs`, Landlock (alla fs-rättigheter, från ABI 4 TCP, från ABI 6 abstrakta socketar och signaler; tyst utan), seccomp-bpf-tillåtelselista: allt annat `EPERM`, `clone` bara med `CLONE_THREAD`, `tgkill` bara till egen pid, `clone3` `ENOSYS`, främmande arkitektur/x32 dödas | samma | ja: sonden nekar alla fyra (Landlock ABI 7 på 6.18), alla flikprocesstester och sviten (55/55 à 0,000 %) kör sandlådade, ingen `EPERM` i strace |
| Windows | skalet: jobbobjekt per flik (`KILL_ON_JOB_CLOSE`, `ACTIVE_PROCESS_LIMIT` 1, minnestak 256 MB + 1 GB, `DIE_ON_UNHANDLED_EXCEPTION`, alla UI-begränsningar); fliken: låg integritet på sin egen token, inga extension points, inga bilder från fjärrresurser eller låg-märkta filer, inga icke-systemtypsnitt | i stället för std:s spawn: `CreateProcessW` i AppContainern `vy.tab` utan capabilities, `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (stdin, stdout, NUL som stderr), `CHILD_PROCESS_RESTRICTED`, startad suspenderad, i jobbet, sedan återupptagen; fliken lägger till strikt handtagskontroll; går det inte: meddelande och standard | bara typkontroll och clippy (`x86_64-pc-windows-msvc`) |
| macOS | fd > 2 stängda, `sandbox_init` med `(deny default)` + signal/process-info till sig själv + `sysctl-read`; misslyckas anropet: varning, osandlådad | samma | bara typkontroll och clippy (`aarch64-apple-darwin`) |

`VY_SANDBOX=off` stänger av barnets låsning (felsökning; jobbobjektet på Windows står kvar).
`vy-shell --tab --sandbox-probe` låser och försöker sedan läsa den egna binären, skriva en fil i
temp, binda en UDP-socket och starta binären; rapporten är en rad per försök och exitkod 0 bara
om allt nekades. `process::probe_process` startar sonden exakt som skalet startar en flik (jobb,
strict), och `spec_9_1_3_tab_sandbox` (vy-shell/tests/sandbox.rs) kör den: Linux kräver alla fyra
nekade när seccomp finns, Windows skrivning och process när tokenen sänkts, macOS alla fyra när
profilen tagits; annars skriver testet varför det hoppar. Testet visar också att `off` är av.

**Windows standard uppfyller SPEC-9.1.3 bara till hälften:** låg integritet hindrar skrivning
till användarens filer och register, jobbet hindrar barnprocesser, men *läsning* av filer och
*nätverk* stoppas bara av AppContainern i strict. Att göra AppContainern till standard kräver
att någon kört den på en riktig maskin (spec-fråga 62).

### Vad människan ska se på Windows

1. **Standard (ingen miljövariabel).** Allt som i testomgång 1. I Aktivitetshanteraren →
   Detaljer, kolumnen "Integritetsnivå" (lägg till via högerklick på rubriken → Välj kolumner):
   `vy-shell.exe` = Medel, varje flikprocess (`vy-shell.exe --tab`) = **Låg**. Process Explorer
   (Sysinternals) visar dessutom flikprocessen i ett jobb (fliken "Job": 1 aktiv process,
   minnesgräns ~1,25 GB). Stäng skalet med Aktivitetshanteraren ("Avsluta aktivitet" på
   huvudprocessen): flikprocesserna ska försvinna direkt (kill on job close), inte först när röret
   stängs.
2. **Sonden.** I en kommandotolk: `vy-shell.exe --tab --sandbox-probe` (utan jobb, eftersom
   skalet inte startar den): väntat `file-write: refused` (låg integritet), `file-read: allowed`,
   `socket: allowed`, `process: allowed` (inget jobb här). Exitkod 1 är väntat i det läget.
3. **Strict.** `set VY_SANDBOX=strict` och starta `vy-shell.exe` från samma kommandotolk. Väntat
   om binären ligger där `ALL APPLICATION PACKAGES` får läsa den (t.ex. under
   `C:\Program Files\Vy\`): sidorna fungerar som förut, flikprocesserna visar Integritetsnivå
   **AppContainer**. Ligger binären i Hämtade filer kan AppContainern inte läsa den: då skriver
   skalet (i en debug-build på konsolen) "VY_SANDBOX=strict: … this tab starts with the default
   sandbox" och fliken startar som i 1. Om sidorna i strict visar "Sidan stannade": rapportera
   det med Windows-version – det betyder att wasmtime eller rören inte klarar AppContainern.
4. **Om något i standardläget går sönder** (flikar startar inte, "Sidan stannade" på allt):
   `set VY_SANDBOX=off` och starta igen; fungerar det då är det barnets låsning (låg integritet
   eller en policy) – rapportera vilket.

### Hotbild – flikprocessens sandlåda

- **Vad.** Ingen ny yta mot sidor; en minskad: processen som kör sidans kod tappar fil-, nät- och
  processrättigheter. Ny kod i skalet: jobbobjekt och AppContainer-start (Windows), en sond.
- **Vem kan missbruka.** Den som tar sig ut ur wasmtime (en bugg i Cranelift eller wasmtimes
  körning) och får kod att köra i flikprocessen; därifrån: läsa användarens filer, nå nätet eller
  andra processer, skriva m4/perf:s kompileringscache, prata med X11/Wayland genom ett ärvt fd.
- **Hur begränsas det.** Se tabellen. Linux: inga öppna, inga socketar, inga exec, inga
  fork-lika clone, inga signaler till andra, inga ärvda fd utöver 0–2; Landlock under. Windows:
  ingen skrivning till medelnivå (kompileringscachen under `%LOCALAPPDATA%` är medelnivå), inga
  barnprocesser, inget UI; i strict ingen läsning, inget nät. macOS: deny default. Rören och
  minnet är kvar – det är flikens jobb; IPC-avkodaren i huvudprocessen litar inte på fliken
  (granskning 0004 fynd 2).
- **Test som bevisar begränsningen.** `spec_9_1_3_tab_sandbox`; `sandbox::platform::tests::
  filter_layout` (filtrets hopp); alla befintliga flikprocesstester (`spec_9_1_1`, `spec_11_2_1`,
  `spec_11_4_1` utan inspelning, `spec_13_3_1`, skalfallen i sviten) kör sandlådade.

## Steg 5 – resursgränser (SPEC-9.3.1) och isolering per origin (SPEC-9.5.1)

Läget före: `memory_max` (wasmtimes tillväxtgräns), 8 ms-bränslet med `KIND_BUDGET` och tvingad
`vy_suspend`, 64 öppna anrop (`E_RATE`, `vy_abi::limits::MAX_OPEN_FETCHES`) och `quota_kb`
(`E_QUOTA`) fanns i värden och var testade var för sig; 256 MB/min fanns bara i huvudprocessen
och räknade svaret först efter att det levererats; huvudprocessen hade ingen gräns för antalet
anrop i luften (granskning 0004 fynd 7).

- **Värden (vy-runtime, sidans sida).** URL + begärans kropp räknas vid `fetch_begin`, svarets
  kropp vid `FETCH_DONE`, per minut av sidans klocka (`NET_BYTES_PER_MINUTE`, exporterad). Ett
  anrop som inte ryms är `E_RATE` och görs aldrig; ett svar som inte ryms levereras som
  `FETCH_DONE` med `b = E_RATE` och ingen kropp och stänger anropet (som `E_TOO_LARGE` över 64
  MB). Ingen ändring i ABI:t: `E_RATE` är redan koden för "för många" (SPEC-5.2.7).
- **Huvudprocessen (vy-shell, förtroendegränsen).** `netlimit::NetBudget` per flik: högst 64 i
  luften och 256 MB per minut av skalets klocka; det 65:e anropet och byte över minuten nekas med
  `E_RATE` innan laddaren ser dem, ett svar över minuten levereras utan kropp. Byte-fönstret
  följer fliken över navigeringar (en sida kan inte nollställa det genom att ladda om sig), antalet
  i luften följer sidan. `shell.rs` sätter dessutom Windows-jobbets minnestak efter sidans
  `memory_max` (`TabConnection::limit_memory`).
- **Per origin.** Lagringen på disk är per origin sedan M3 (`storage.rs`, SHA-256-namngiven fil,
  `spec_9_5_2_storage_is_per_origin_on_disk`); grants och förtroende är per origin i vy-proto och
  skalet. `spec_9_5_1_per_origin_isolation` bevisar värdens del: varje sidinstans har sin egen
  `Storage`, en nyckel som en origin skriver finns inte för en annan, och en senare instans av
  samma origin läser det den fått.

Un-ignorerade: `spec_9_3_1_resource_limits` (alla fem gränserna på en gång, 256 MB/min med fyra
svar à 64 MB och en minut framåt på klockan) och `spec_9_5_1_per_origin_isolation`. Nytt:
`spec_9_3_1_main_process_net_limits`. Kvar ignorerat i vy-runtime: `spec_6_2_2_idle_is_free`
(bevisas på skalnivå av SPEC-13.3.2-testet).

### Hotbild – resursgränser

- **Vad.** Ingen ny yta; två räknare (värd och huvudprocess) framför en befintlig.
- **Vem kan missbruka.** En sida som vill göra klienten till en nedladdnings- eller
  överbelastningsmaskin (mot sin egen origin eller en origin den fått i `net`), eller en
  komprometterad flikprocess som skickar `FetchRequest` förbi sin egen värd.
- **Hur begränsas det.** Huvudprocessen räknar själv och litar inte på flikens räkning; en
  sida kan inte nollställa minuten genom navigering; svaret räknas *innan* det levereras.
- **Test som bevisar begränsningen.** `spec_9_3_1_resource_limits`,
  `spec_9_3_1_main_process_net_limits`, `spec_5_2_8_main_process_gates_page_requests` (oförändrat
  grönt).

## Flikprocesser startas från den körande avbilden (`process::self_exe`)

Koordinatorns uppdrag efter fyndet att `target/debug/<bin>` delades mellan arbetskopior: samma
fel finns i produkten. En `vy-shell` som byts ut på disk medan den kör (en uppdatering) startade
nya flikar från den nya binären, kanske med ett annat IPC-protokoll. `self_exe()` ger
`/proc/self/exe` på Linux (barnet exekverar den innan det fått en egen avbild, så den namnger
förälderns inod) och `current_exe()` annars: Windows låter inte en körande binär ersättas (bara
byta namn, och namnbytet behåller avbilden); på macOS står risken kvar. Används vid
`tab_executable`, i fönstret, i `--screenshot` och i vy-cli:s konformanskörare.

**Hotbild.** *Vad:* vilken binär en flikprocess blir. *Vem kan missbruka:* den som kan skriva över
binären medan klienten kör (en uppdaterare, en installation, en annan användare med skrivrätt
till katalogen) fick nya flikar att köra sin kod utan omstart av skalet. *Hur begränsas det:* nya
flikar kör alltid skalets egen avbild. *Test:* `spec_9_1_1_tabs_start_the_running_image` – en
kopia av testbinären byts mot ett skript medan den kör, och barnet den sedan startar är
fortfarande originalet; testet faller utan fixen (provat).

## Steg 6 – fuzzning

`fuzz/` är en egen cargo-workspace (en `[workspace]`-tabell, alltså inte medlem i huvudytan och
inte byggd av `cargo test --workspace`) med cargo-fuzz-målen `manifest` (kanonisk CBOR med
omkodningskontroll, nyckelloggens post och textform, paketet), `display_list`, `ipc` (båda
riktningarna, ett accepterat meddelande kodar om till samma byte, ramläsaren), `address`
(`vy://` med rundtur genom texten, URL-upplösning), `png`, `markdown` och `dns` (svarstolken med
id ur de två första byten, frågebyggaren). Ingångarna ligger i `fuzz/src/entry.rs`.

**Stabil väg.** `crates/vy-shell/tests/fuzz_regressions.rs` inkluderar samma `entry.rs` (`#[path]`)
och kör (a) varje fil i `fuzz/corpus/<mål>/` och `fuzz/artifacts/<mål>/`, (b) 250 deterministiska
mutationer (bitflipp, extrema byte, trunkering, dubblering, insättning) av de sex första seeds per
mål. En krasch som nattjobbet hittar läggs in under `fuzz/artifacts/<mål>/` i samma commit som
fixen och är därefter en grind i `cargo test`. `VY_FUZZ_SMOKE_ROUNDS` och `VY_FUZZ_SMOKE_SEED`
kör fler; `VY_WRITE_FUZZ_SEEDS=1` skriver om de 49 seeds (de elva manifesten signerade med en
testnyckel, hello-paketet, en logg, display lists från hello/shop/text/dashboard, elva
IPC-meddelanden, fem adresser, de två minsta PNG:erna i gallery, elva reservtexter, två DNS-svar).

**Nattjobbet.** `.github/workflows/fuzz.yml`: kl. 02:17 UTC och manuellt (`seconds`), en matris
med sju mål, `RUSTUP_TOOLCHAIN=nightly` förbi `rust-toolchain.toml`, `cargo install cargo-fuzz`,
`cargo fuzz run --fuzz-dir fuzz <mål> fuzz/corpus/<mål> -- -max_total_time=300 -rss_limit_mb=2048
-timeout=10`, krascher som artefakt. Inte kört (ingen CI härifrån); YAML:en parsar.

**Lokalt.** Ingen libFuzzer-körning: nightly + ASan-bygge av hela trädet (wasmtime) hade tagit
flera GB på en disk med 7–15 GB fritt delat av fyra spår. I stället körde mutationspasset
3 × 30 000 runder per seed (~3,6 M indata): **ett fynd**, se nedan, inget annat.

**Fynd.** `KeyLog::parse` (mitt eget, steg 3) skar loggraden med `&s[i..i + 2]` och panikade på
ett tecken över en byte – nåbart från varje origin som serverar en logg, vid första besöket.
Fixat i en egen commit (hex läses som byte); regressionsfallet står i
`spec_7_6_1_log_format_and_chain` och som `fuzz/artifacts/manifest/log-multibyte-hex`.

## Checklista för den externa sandlådegranskningen (utkast)

För en granskare utifrån som ska försöka bryta processgränsen och förtroendekedjan. Varje punkt:
vad som ska försökas, hur, vad som räknas som fynd.

1. **Ut ur wasmtime.** Anta en kodkörning i flikprocessen (skriv en liten Rust-binär som länkar
   `vy_shell::sandbox`, kör `lockdown` och sedan valfria anrop – eller använd
   `vy-shell --tab --sandbox-probe` som mall). Försök: öppna, `stat`a eller `readlink`a filer,
   `openat2`, `io_uring_setup`, `socket`/`socketpair`, `connect` till en abstrakt unix-socket,
   `execve`/`execveat`, `fork`/`vfork`/`clone` utan `CLONE_THREAD`, `clone3`, `ptrace`,
   `process_vm_writev`, `kill`/`tgkill` mot skalets pid, `prctl`, `personality`, `userfaultfd`,
   `perf_event_open`, `bpf`, `memfd_create` + `execveat`, `mmap` av ett ärvt fd. Fynd: något
   som lyckas utöver tillåtelselistan i `sandbox.rs`.
2. **Ärvda handtag.** Starta skalet under en process som läcker fd utan `O_CLOEXEC` (t.ex. en
   X11-/Wayland-anslutning, en socket) och se om fliken har dem (`ls -l /proc/<flik>/fd`).
   Windows: `handle.exe -p <flik>` – i strict ska bara rören och NUL finnas. Fynd: något utöver
   0, 1, 2 (+ memfd från wasmtime).
3. **Windows standardläge.** Bekräfta låg integritet och jobbet (Process Explorer). Försök från
   fliken: skriva i `%USERPROFILE%`, `%LOCALAPPDATA%` (m4/perf:s kompileringscache), HKCU;
   starta en process (jobbets gräns 1); läsa användarens filer och öppna en socket – **väntat
   möjligt i standard**, nekat i strict. Fynd: skrivning eller process.
4. **Windows strict (AppContainer).** Med `VY_SANDBOX=strict` och binären under Program Files:
   flikprocessen i AppContainern `vy.tab`, inga capabilities; försök filer, nät, named pipes,
   COM/RPC, `\Device\Afd`, andra processers handtag. Fynd: allt som lyckas; och om strict
   tyst faller tillbaka till standard när det borde fungera.
5. **macOS.** Kör sonden; bekräfta att `sandbox_init` tog profilen (raden `sandbox:`); prova
   Mach-tjänster (`bootstrap_look_up`), filer, nät. Fynd: allt som lyckas utöver signaler och
   process-info till sig själv.
6. **Huvudprocessen som motpart.** Skicka fientliga `ToShell`-ramar från en egen flikprocess
   (IPC-avkodaren är fuzzad, `ipc`-målet): `FetchRequest` utan `net`, till främmande origin, fler
   än 64 i luften, över 256 MB/min; `Storage` över kvoten; `Navigate` till `file:`, `http:`,
   en lokal sökväg. Fynd: något som når nätet eller disken utan att huvudprocessen beslutat det.
7. **Loopback och privata nät (SPEC-9.5.3, m4/dev, granskning 0007 fynd 3).** Regeln prövas
   efter namnuppslaget på varje anslutning för en sida: en sida vars egen adress är offentlig når
   bara offentliga adresser; en sida på en privat eller länklokal adress (`10/8`, `100.64/10`,
   `172.16/12`, `192.168/16`, `169.254/16`, `fc00::/7`, `fe80::/10`) når privata och offentliga
   men aldrig loopback (`127/8`, `::1`, `0.0.0.0/8`, `::`); bara en sida på loopback når allt.
   Försök från en offentlig sida att nå loopback och det lokala nätet, också över `https://`:
   via `fetch_begin` (`net`-origins som `https://127.0.0.1:…`, `localhost`, `[::1]`, `0.0.0.0`,
   `[::]`, `127.1`, decimala och oktala IP-former, `::ffff:127.0.0.1` och andra IPv6-former av
   IPv4, `169.254.169.254`, DNS-namn som pekar på 127.0.0.1 eller 10.x – DNS-rebinding, också ett
   namn som byter adress mellan klassningen och anslutningen, och ett namn med både en offentlig
   och en lokal adress), via omdirigeringar (alla tre hoppen), via
   `module.url`/`resources[].url` i manifestet, och samma sak från en sida på ett privat nät mot
   loopback. Plain http: bara en sida vars *egen* adress är ett loopback-namn får tala http, och
   bara mot sin egen loopback-origin (SPEC-7.1.3, granskning 0003 fynd 5). `navigate` laddar målet
   som en ny sida med målets egen klass (som en adress användaren skrivit); det är avsett. Fynd: en
   anslutning (en SYN räcker, se med `ss`/`tcpdump`) till en adress som regeln nekar.
   **Läge i skalet:** skalet skickar i dag bara `is_localhost` för sidans egna anrop, alltså
   bedöms en sida på ett privat nät som offentlig där (stängt, men en LAN-sida når inte sin egen
   origin) tills skalet skickar `Fetched::net_class()`.
8. **Förtroendekedjan.** Första besök: förfalskade UDP-svar (fel id, fel fråga, trunkerat,
   CNAME-kedjor till främmande namn), en resolver som svarar SERVFAIL eller långsamt; rotation
   utan att nyckeln står i vy-key eller loggen; en logg som skrivits om, tagits bort eller visar
   olika innehåll för olika klienter (split view, spec-fråga 61); `next_key_id`-injektion i
   nyckelförrådet. Fynd: förtroende utan att både vy-key, DNS (och loggen när den finns) stämmer.
9. **Uppdatering under körning.** Byt `vy-shell` på disk medan den kör och öppna en ny flik
   (Linux: ska vara den gamla avbilden; macOS: känd risk; Windows: går inte att byta).
10. **Resurser.** En sida som allokerar till `memory_max`, bränner bränsle, öppnar 64 anrop,
    fyller minuten, skriver till kvoten – och samtidigt i tio flikar: skalet ska förbli
    responsivt och huvudprocessens minne begränsat.

## Privata nät efter namnuppslag (SPEC-9.5.3, granskning 0007 fynd 3)

Före: `transport_allowed` släppte `https://` till vilken värd som helst, så en fjärrsida som fått
`net` mot `https://127.0.0.1:<port>` (eller ett namn som pekar dit) fick klienten att ansluta till
användarens maskin; TLS skyddade innehållet men inte anslutningen. Nu (`vy-proto/src/net.rs`)
sitter regeln i resolvern för varje ureq-agent: adresserna som uppslaget ger filtreras på sidans
klass, och blir inget kvar är svaret `Error::PrivateNetwork` utan att något ansluts. En agent per
regel (fem, över en delad connector), så omdirigeringar, blobbar, nyckelfil, logg och sidans egna
anrop prövas likadant. Sidans klass är `Loopback` för loopback-namnen (SPEC-7.1.3) och annars den
mest offentliga klass dess värd slås upp till (`Fetched::net_class`).

- **Vad.** Ingen ny yta; en ny gräns för en befintlig: vart klienten ansluter för en sida.
- **Vem kan missbruka.** En fjärrsida som vill nå tjänster på användarens maskin eller LAN
  (routrar, skrivare, molnets metadata på 169.254.169.254) genom `net`, en omdirigering eller en
  blob-URL; en angripare som styr DNS för sitt eget namn (rebinding).
- **Hur begränsas det.** Klassen prövas på adressen efter uppslaget, inte på namnet; IPv4 i
  IPv6-form bedöms som IPv4; ett namn med flera adresser behåller bara de tillåtna. Mot rebinding
  av sidans *eget* namn: första hoppet av sidans hämtning ansluter bara till en adress i den klass
  sidan fått eller en mer lokal (en sida som klassats privat kan inte serveras från en offentlig
  adress), och HTTP-cachens nyckel bär klassen, så ett svar hämtat under en klass återanvänds inte
  under en annan. Kvar: en anslutning som redan ligger i ureq:s pool återanvänds utan ny
  anslutning, men resolvern prövas vid varje anrop och poolen är per regel.
- **Test som bevisar begränsningen.** `tests::spec_9_5_3_private_network_rule` (offentlig sida mot
  loopback, `[::1]`, `::ffff:127.0.0.1`, `0.0.0.0`, namn som pekar på loopback, LAN, länklokalt
  och `fd00::/8` nekas utan att något når servern; omdirigering från en offentlig adress till
  127.0.0.1, till `[::ffff:127.0.0.1]` och till LAN nekas; privat sida når privat och offentligt
  men inte loopback; sida på localhost når sin egen loopback och allt annat; ett namn som klassas
  privat och sedan pekar ut nekas på första hoppet), `net::tests::spec_9_5_3_address_classes`
  (intervallens gränser).

## Tester, backlogg och spec

**Ignorerade → gröna:** `spec_7_4_2_dns_txt_must_agree` (vy-proto), `spec_9_3_1_resource_limits`
och `spec_9_5_1_per_origin_isolation` (vy-runtime). Backloggen 19 → 16. Två tester infördes som
ignorerade i sina `spec:`-commits och un-ignorerades i nästa: `spec_7_6_2_7_6_3_key_log_checked_at_key_decisions`,
`spec_9_1_3_tab_sandbox`.

**Nya tester:** `spec_7_4_5_dns_txt_record_form` (vy-manifest), `dns::tests::spec_7_4_2_txt_wire_format`,
`dns::tests::resolv_conf_nameservers`, `dns::tests::system_resolver_udp_then_tcp` (vy-proto),
`spec_7_4_5_dns_records_for_the_operator` (vy-serve), `spec_7_6_1_log_format_and_chain`,
`spec_7_6_1_log_served`, `spec_7_6_1_log_append`, `sandbox::tests::probe_text_lists_attempts`,
`sandbox::platform::tests::filter_layout`, `spec_9_3_1_main_process_net_limits`,
`process::tests::spec_9_1_1_tabs_start_the_running_image` (med hjälptestet `self_exe_child_stage`,
tomt i en vanlig körning), `fuzz_corpora_replay`, `fuzz_mutation_smoke`. Utökade:
`spec_7_4_4_key_rotation`, `keys::tests::pins_and_successors_persist`; tre tester som räknar
förfrågningar vid första besök räknar nu loggen.

**Spec-id:** 132 → 138 (7.4.5, 7.4.6, 7.6.1, 7.6.2, 7.6.3, 9.1.3), alla spårade; inget i §5,
bilaga A eller C rört. Formen på `spec:`-commits: de tre första (7.4.5, 7.4.6, 7.6.1) bär
specmeningen tillsammans med koden och testet som refererar den, som de tidigare `spec:`-commitsen
i historiken; de två sista (7.6.2–7.6.3, 9.1.3) är bara SPEC.md plus ett ignorerat test som
refererar id:t, och implementationen un-ignorerar det i nästa commit.

**Grindar vid sista commit:** fmt, clippy (även `--target x86_64-pc-windows-msvc` och
`aarch64-apple-darwin` för vy-shell, vy-cli, vy-proto), `cargo test --workspace` 271 gröna / 16
ignorerade (vy-render-benchens 1 s-tak föll under last och gick igenom ensam), spec_trace 138/138,
modulbygge + check-abi + test_modules, konformans 55/55 à 0,000 %. Storlek (SPEC-12.3.1):
`cargo build --release -p vy-shell` här gav 26,9 MB *ostrippad* – den lokala
`.cargo/config.toml` (koordinatorns, `codegen-units = 14` per arbetsytecrate i release) verkar
göra att `strip = true` inte slår igenom; samma binär strippad är 22,1 MB (main: 21,8 MB med
repots egen profil). Grenens tillägg (DNS, logg, sandlåda) är alltså ~0,3 MB eller mindre;
CI bygger utan den lokala filen.

## Kvar

- Windows strict (AppContainer) och macOS-profilen är bara typkontrollerade; Windows standard
  stoppar inte läsning och nät (spec-fråga 62).
- Ingen libFuzzer-körning lokalt; nattjobbet har aldrig kört.
- Skalet visar inte `Fetched::restriction()` (m4/ux).
- `limit_memory` tar sidans `memory_max` vid spawn; ingen gräns på Linux/macOS utöver wasmtimes
  egen (RLIMIT_AS skulle bryta wasmtimes reserverade adressrymd).
- Split view i loggen (spec-fråga 61) och Railway-domänen (steg 1).

## Byggmiljön (för den som tar över)

- `target/` var först delad mellan fyra arbetsträd. Cargos metadatahash för en sökvägscrate
  räknas relativt arbetsytans rot, så samma crate i två arbetsträd fick samma artefakt och ett
  träd kunde länka ett annat träds bygge (observerat: vy-proto länkade en vy-manifest utan de nya
  funktionerna); de upplyfta binärerna `target/debug/vy-shell`, `vy-cli` delades också, även
  medan `cargo test` körde (observerat: konformansens skalfall 15–87 % fel, sandlådetestet fick
  en annan arbetskopias vy-shell). Koordinatorn gav varje arbetsträd en ospårad
  `.cargo/config.toml` (19:31; checka inte in den) och sedan 22:16 en egen target-katalog
  (`target` → `/home/claude/vy-target-security`). Röda tester före 22:16 som startade en binär
  är omkörda. `target/wasm` byggs med `tools/build_modules.sh` i den nya katalogen.
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
- **Steg 5 och självstart:** `crates/vy-runtime/src/host.rs` och `lib.rs` (m4/perf):
  `NET_BYTES_PER_MINUTE`, `Host::net_admit`, ett fält `net_minute`, en kontroll i
  `fetch_begin` och en gren i `deliver_fetch` (omstrukturerad utan att ändra beteendet för
  övriga svar). `crates/vy-shell/src/shell.rs` (m4/ux): `net_window` är nu
  `netlimit::NetBudget` (fältet, initieringen, `fetch_for`, svarsgrenen, nollställning vid
  navigering), konstanten `NET_BYTES_PER_MINUTE` borttagen därifrån, och `limit_memory` efter
  spawn. Ny fil `src/netlimit.rs`, `pub mod netlimit;` i `lib.rs`. `window.rs:175` och
  `main.rs` (`--screenshot`) och `crates/vy-cli/src/conformance.rs` (`tab_executable`): en rad
  var, `self_exe()`.
- **Steg 6:** ny fil `crates/vy-shell/tests/fuzz_regressions.rs` (läser `fuzz/src/entry.rs`).
- `crates/vy-shell/Cargo.toml` (m4/ux): `libc` för unix och fler windows-sys-features (samma
  crates som fanns). `src/lib.rs`: `pub mod sandbox;`. `src/main.rs`: `--tab` tar
  `--sandbox-probe` och låser innan `tab::main`. `crates/vy-cli/src/main.rs` (m4/perf): samma
  tre rader i `--tab`-grenen. `process.rs` (min) fick `probe_process` och
  `TabConnection::limit_memory(memory_max_pages)`: **m4/ux bör anropa den när `Load` skickas**
  (shell.rs känner manifestets `memory_max`), så att Windows-jobbets tak blir sidans
  `memory_max` + 1 GB i stället för 256 MB + 1 GB.
- **m4/perf (kompileringscache):** flikprocessen kan inte öppna någon fil efter uppstart (Linux,
  macOS) och inte skriva på medelnivå (Windows). En cache måste alltså läsas och skrivas av
  huvudprocessen och nå fliken över IPC; läggs den under `%LOCALAPPDATA%Low` på Windows kan
  en flik med låg integritet skriva den – gör inte det.
- **m4/dev (loopback):** inget i det här spåret låter en sida utanför loopback nå en
  loopback-origin: nyckelfil, DNS och logg hämtas för sidans egen adress, http bara när sidans
  egen adress är loopback (granskning 0003 fynd 5); DNS-uppslaget går till systemets resolver
  (kanske 127.0.0.53) men sidan väljer bara namnet `_vy.<värd>`, aldrig servern. Regeln står i
  checklistan för den externa granskningen.
- **m4/ux (vy-shell):** `vy_proto::Fetched::restriction()` finns nu; SPEC-7.4.5 vill att orsaken
  kan visas – t.ex. som verktygstips eller rad under markören "begränsat läge". `page.rs`
  kopierar i dag bara `trusted`.

## Spec-frågor

58. **SPEC-7.4.5 – fler än en nyckel.** Första besöket pinnar alla nycklar som står i båda
    källorna, inte bara den som signerat sidan (som M3 gjorde med nyckelfilen). En utgivare
    med två aktiva nycklar behöver alltså två TXT-poster. Avsett?
59. **SPEC-7.4.5 – väntetid.** Ett uppslag som inte svarar tar upp till 6 s och sker i skalets
    laddartråd (som är seriell, granskning 0004 fynd 7). Bör specen ge en tidsgräns, och bör
    sidan visas i begränsat läge medan uppslaget pågår?
60. **SPEC-7.4.5 – IP-literaler.** `vy://192.0.2.1/` frågar `_vy.192.0.2.1` och faller till
    begränsat läge. Det finns ingen DNS-zon för en adress; ska IP-adresser (utom loopback) vara
    otillåtna i `vy://`, eller alltid begränsade?
61. **§7.6 – split view.** En server kan visa olika loggar för olika klienter. Bara speglar som
    jämför huvuden upptäcker det; ska 1.0 ha en mekanism (en andra källa för huvudet, t.ex. ett
    huvud i DNS bredvid TXT-posten), eller räcker det att formatet är speglingsbart?
62. **SPEC-9.1.3 på Windows.** Standardläget hindrar skrivning och barnprocesser men inte läsning
    och nätverk; AppContainern (strict) hindrar allt men är inte provad. Bör meningen gälla
    strict, och standard bli strict när en människa kört det?
