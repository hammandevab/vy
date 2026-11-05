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
| 3 | Transparenslogg (ny §7.6, SPEC-7.6.1–7.6.3) | klart |
| 4 | OS-sandlåda för flikprocessen (SPEC-9.1.1, 9.1.2, 11.2.2, ny 9.1.3) | klart (Linux provat, Windows/macOS typkontrollerat) |
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
  jämför huvuden – utanför 1.0 (spec-fråga 56). (3) En fientlig logg mot tolken: storlek,
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
att någon kört den på en riktig maskin (spec-fråga 57).

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

53. **SPEC-7.4.5 – fler än en nyckel.** Första besöket pinnar alla nycklar som står i båda
    källorna, inte bara den som signerat sidan (som M3 gjorde med nyckelfilen). En utgivare
    med två aktiva nycklar behöver alltså två TXT-poster. Avsett?
54. **SPEC-7.4.5 – väntetid.** Ett uppslag som inte svarar tar upp till 6 s och sker i skalets
    laddartråd (som är seriell, granskning 0004 fynd 7). Bör specen ge en tidsgräns, och bör
    sidan visas i begränsat läge medan uppslaget pågår?
56. **§7.6 – split view.** En server kan visa olika loggar för olika klienter. Bara speglar som
    jämför huvuden upptäcker det; ska 1.0 ha en mekanism (en andra källa för huvudet, t.ex. ett
    huvud i DNS bredvid TXT-posten), eller räcker det att formatet är speglingsbart?
57. **SPEC-9.1.3 på Windows.** Standardläget hindrar skrivning och barnprocesser men inte läsning
    och nätverk; AppContainern (strict) hindrar allt men är inte provad. Bör meningen gälla
    strict, och standard bli strict när en människa kört det?
55. **SPEC-7.4.5 – IP-literaler.** `vy://192.0.2.1/` frågar `_vy.192.0.2.1` och faller till
    begränsat läge. Det finns ingen DNS-zon för en adress; ska IP-adresser (utom loopback) vara
    otillåtna i `vy://`, eller alltid begränsade?
