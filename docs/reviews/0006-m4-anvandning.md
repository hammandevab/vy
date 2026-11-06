# PR #6 – M4: användning – urklipp, lagring, markering, sök, AccessKit, devtools (`main..m4/ux`, topp `976b3a1`, bedömd på `ccef8b4`)

**Vad ändrades.** Systemets urklipp bakom Ctrl+C/V/X, med en kopieringsgest som låter en sida skriva
urklippet utan capability (ny SPEC-8.2.4). Lagringen sparas nu vid F5, navigering och Ctrl+W.
Tillkommer: värdens egen textmarkering, sök på sidan (Ctrl+F), ett AccessKit-träd för hjälpmedel
och devtools (F12) med display list, semantik, logg, inspelning och live-omladdning för localhost.
6 600 rader, 24 filer.

**Spec-id som berörs.** Ny 8.2.4. Un-ignorerade: 10.2.1, 10.2.2, 10.3.1, 10.4.1. Berör 5.2.9, 5.2.12,
6.2.1, 7.3.2, 11.4.1. Spec-frågor 55–57.

**Risk.** Urklippet och lagringen är användarens data. Fel här betyder att en sida läser eller
skriver urklippet utan att användaren bett om det, eller att inställningar försvinner. Devtools
startar flikar på ett nytt sätt (inspelning), som går förbi sandlådan (0007, fynd 1).

**Bevis.** De citerade testerna är gröna på HEAD, bland dem `spec_8_2_4_copy_gesture_grants_clipboard_write`,
`spec_5_2_12_clipboard_copy_paste_through_the_shell`, `spec_5_2_9_storage_saved_on_reload_and_close`,
`spec_10_2_2_text_selection`, `spec_10_2_1_accesskit` och `spec_11_4_1_devtools_record_and_replay`.
Inga guldbilder ändrade. Kontrollerat i koden:
*Kopieringsgesten* armas bara av en riktig Ctrl+C/X med fokus i sidan och utan värdmarkering
(`Shell::key`, `copy_gesture`). Flikprocessen släpper `clipboard_write` bara under just det
`KEY_DOWN` (`Runtime::event_as_copy_gesture`). Huvudprocessen kräver gesten igen, inom 1 s och en
skrivning per gest (`take_copy_gesture`). Läsning kräver fortfarande `clipboard` och en gest. Ingen
väg hittad att skriva utan tangenten eller att läsa. Fynd, efter allvar:

1. **Låg – en avslutande flik håller UI-tråden.** `shell.rs`: `navigate_tab`, `close_tab` och
   `Shell::shutdown` går genom `retire_process` → `TabConnection::shutdown`, som väntar upp till
   500 ms på UI-tråden innan processen dödas. Vid fönsterstängning görs det en flik i taget.
   Väntan fanns i M3, men nu bär den lagringens tömning. En sida som bränner bränsle till taket i
   `vy_suspend` och navigerar till sin egen adress varje gång den laddas gör skalet trögt; tio sådana
   flikar fördröjer stängningen. (Från koden, inte uppmätt.) Rättning: skicka `Shutdown` till alla
   flikar på en gång med en gemensam tidsgräns, och flytta väntan från UI-tråden. Låt laddarens
   nästa jobb för fliken vänta på den pensionerade processens svar.
2. **Låg, beslut för ägaren – Ctrl+V ger varje sida urklippet.** `paste_into_page` levererar upp
   till 1 MB som `TEXT_INPUT`, utan capability och även i begränsat läge. Det är avsiktligt och
   dokumenterat (som webbens `paste`), men en sida kan be användaren "tryck Ctrl+V". Bör stå i
   SPEC-8.2.4 eller 5.2.12 så att det är ett beslut och inte bara kod.
3. **Låg – inspelning i devtools kör fliken utan sandlåda.** Se 0007, fynd 1. Panelen bör säga det
   tills det är rättat.

**`a11y-linux` och 25 MB.** Med AT-SPI blir `vy-shell` 25 435 064 byte. Det är över 25 MB räknat
decimalt men under 25 MiB (talet är integrationsagentens; jag har inte byggt om det). Rekommendation:
**låt funktionen vara av som standard** och läs SPEC-12.3.1 decimalt, det strängare. För att slå på
den behöver ~0,5 MB tas någon annanstans först. Kandidater: `pulley_interpreter` (0,2 MB, används
inte, princip 6) och typsnitten (2,2 MB, delmängd). Ägaren avgör spec-fråga 56. Linux-användare
med skärmläsare får tills dess inget träd, och det bör stå i release-texten.

**Nya beroenden.** `arboard` 3.7 (utan bild och Wayland), `accesskit` 0.25, `accesskit_winit` 0.34;
motiverade i notes/m4-ux. `a11y-linux` drar +80 crates och är av.

**Hotbild.** Ingen ny ABI-import. Nya ytor: systemets urklipp (ovan), plattformens
tillgänglighets-API (sidans text, som avsett; en fientlig föräldrarelation kan inte ge ett träd
utan rot, enligt `spec_10_2_1_any_tree_is_well_formed`), och inspelningsfiler i datakatalogen.

**Rekommendation.** **Merge.** Fynd 1–3 som uppföljningar. Fynd 3 rättas under 0007.
