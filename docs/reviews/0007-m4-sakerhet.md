# PR #7 – M4: säkerhet – förtroendekedjan, flikens sandlåda, gränser, fuzz (`main..m4/security`, topp `b3a5b7b`, bedömd på `ccef8b4`)

**Vad ändrades.** Första besöket av en origin kräver att nyckeln också står i DNS TXT (`_vy.<värd>`,
SPEC-7.4.5). Rotation pensionerar den gamla nyckeln (7.4.6). En transparenslogg med spärrhake
tillkommer (§7.6), liksom en OS-sandlåda för flikprocessen (9.1.3), nätgränser i huvudprocessen
(9.3.1), `self_exe` för nya flikar och sju fuzzmål med en stabil mutationsväg i `cargo test`.
5 800 rader, 93 filer.

**Spec-id som berörs.** Nya: 7.4.5, 7.4.6, 7.6.1–7.6.3, 9.1.3. Un-ignorerade: 7.4.2, 9.3.1, 9.5.1.
Spec-frågor 58–62.

**Risk.** Det här är förtroendegränsen. Fel ger förtroende utan att nyckelfil, DNS och logg stämmer,
eller kod från en sida med användarens rättigheter.

**Bevis.** *Sandlådan, Linux (defensivt, ur `sandbox.rs`).* Tillåtelselistan har 36 anrop + `arch_prctl`:
read, write, readv, writev, close, lseek, mmap, mprotect, munmap, mremap, madvise, brk, memfd_create,
ftruncate, fcntl, rt_sigaction/-procmask/-return, sigaltstack, getpid, gettid, getrandom,
clock_gettime/-getres/-nanosleep, nanosleep, sched_yield, sched_getaffinity, futex, set_robust_list,
rseq, membarrier, prlimit64, restart_syscall, exit, exit_group. Med villkor: `clone` bara med
`CLONE_THREAD`, `tgkill` bara till den egna pid:en, `clone3` → `ENOSYS`. **Saknas, som det ska:**
execve/execveat, fork/vfork, ptrace, process_vm_readv/writev, open/openat/openat2/creat, socket,
connect, bind, mount, bpf, perf_event_open, keyctl, userfaultfd, io_uring. Nekat anrop → `EPERM`.
Främmande arkitektur och x32 → processen dödas. Under filtret ligger Landlock utan rättigheter
(ABI 7 här), `no_new_privs`, `TSYNC`, och fd > 2 stängs. `spec_9_1_3_tab_sandbox`: alla fyra
försök (läsa, skriva, socket, process) nekas med `EPERM`. Låsningen sker i `main.rs` före
`tab::main`, alltså före första IPC-ramen, med ett undantag: `--record`/`--replay` (fynd 1).
*Windows standard:* jobbobjekt (död vid stängning, en process, minnestak, UI-gränser) + låg
integritet + tre processpolicyer (inga extension points, inga bilder från fjärrresurser eller
låg-märkta filer, inga icke-systemtypsnitt). Det hindrar skrivning till användarens filer och register,
barnprocesser och UI, men **inte läsning av filer eller nätverk**. Strict (AppContainer) är bara
typkontrollerat. *DNS:* UDP ≤ 4 096 B med en total tidsgräns per försök; fel id ignoreras; ≤ 64
poster, ≤ 8 CNAME, namn ≤ 253. *Loggen:* ≤ 1 MB före tolkning, strikt avkodning; fuzzens enda fynd
(panik på flerbytetecken) är rättat. *Kompileringscachen:* nyckeln är SHA-256 över format,
wasmtimes `precompile_compatibility_hash` (version och alla inställningar) och modulens hash;
nyttolasten kontrolleras och läses hel; katalog 0700. Den är **av i skalet**, så varken flik eller
sida påverkar den i dag. *Nätgrinden:* en tom eller udda `net`-lista når inget i alla tre lager
(spec-fråga 63 kvar). Plain http når bara sidans egen loopback-origin. Omdirigeringar prövas per
hopp; en sidas `fetch_begin` följer inga. Fynd, efter allvar:

1. **Medel – `--record`/`--replay` kör fliken utan OS-sandlåda.** `vy-shell/src/main.rs`: lockdown
   bara om båda saknas. Devtools "Inspelning" startar en flik så, på användarens klick. Sidan har
   då bara wasmtime mellan sig och användarens filer och nät. Rättning: öppna filen före låsningen
   och behåll fd:t (stäng från fd + 1), eller låt huvudprocessen skriva inspelningen ur IPC-strömmen;
   vid uppspelning läses hela filen först. Test: kör sonden med `--record`.
2. **Medel – DNS över TCP saknar total tidsgräns.** `vy-proto/src/dns.rs`, `system::tcp`:
   `set_read_timeout(2 s)` gäller varje läsning, och `read_exact` läser upp till 65 535 byte. En
   resolver som droppar en byte i taget håller uppslaget långt över noteringens "högst 6 s". Det
   sker på laddartråden, som laddar alla flikars sidor. Repro (enhetstest): en TCP-lyssnare som
   svarar längd 0xFFFF och sedan en byte per 1,5 s. Rättning: en tidsgräns för hela `tcp()`, som i
   `udp()`.
3. **Låg till medel – `https://` till loopback och LAN är tillåtet för fjärrsidor.**
   `fetch.rs`, `transport_allowed`: https släpps alltid igenom. TLS skyddar innehållet, men en
   fjärrsida kan få klienten att ansluta till portar på användarens maskin. Checklistans punkt 7
   säger motsatsen. Rättning: neka loopback (och gärna privata adresser) efter namnuppslag för
   sidor utanför loopback, eller ändra checklistan.
4. **Låg – `prlimit64` utan argumentkontroll.** Efter en utbrytning ur wasmtime kan fliken ändra
   gränser för användarens andra processer, skalet inräknat. Rättning: tillåt bara pid 0 eller den
   egna, som `tgkill`.
5. **Låg – sandlådan faller öppen i det tysta.** Misslyckas seccomp, Landlock, `sandbox_init`,
   jobbobjektet eller låg integritet blir det bara en rad på stderr, och en GUI-binär på Windows har
   ingen konsol. Rättning: rapportera till skalet och visa det i chromen.
6. **Låg – `compile_cache.rs`, `dir_is_private`, prövar läget men inte ägaren.** Lägg till en uid-kontroll
   innan cachen slås på.

**Nya beroenden.** Inga nya crates; `getrandom`, `windows-sys` (två funktionsgrindar), `libc` fanns.

**Hotbild.** Ingen ny ABI-import; hotanalyserna per steg står i notes/m4-security och håller med
fynden ovan. Railway-sajten visas i begränsat läge vid första besök tills den har en egen domän och
TXT-post (ägarens sak).

**Rekommendation.** **Merge efter** fynd 1 och 2. Fynd 3–6 som uppföljningar. M4:s grind "extern
sandlådegranskning" är inte gjord. Utkastet till checklista för den finns i notes/m4-security.
