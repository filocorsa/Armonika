# Fase 0: spike e impalcatura. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** preparare la repository (workspace Rust, CI su tre sistemi, generatori delle librerie di prova) ed eseguire i 6 spike del §11 del design, chiudendo con una decisione scritta e motivata da numeri per ognuno.

**Architecture:** workspace Cargo con solo le crate che servono adesso (`armonika-core`, `armonika-server`, `armonika-testdata`); le altre crate del §7.5 nascono nelle fasi in cui servono. Gli spike sono progetti Cargo separati in `spikes/`, esclusi dal workspace, usa-e-getta: il loro prodotto è un documento in `docs/spikes/`, non codice.

**Tech Stack:** Rust 1.99 (edition 2024), clap, lofty, rusqlite (FTS5), tantivy, symphonia, rusty-chromaprint, reqwest, serde, GitHub Actions, Docker (MeTube), ffmpeg e fpcalc come strumenti esterni di sviluppo.

**Spec:** `docs/superpowers/specs/2026-11-12-armonika-design.md` (approvato). Chi esegue legge entrambi; il Glossario del design vale anche qui.

## Global Constraints

- Licenza AGPL-3.0; ogni crate dichiara `license = "AGPL-3.0-only"` in `Cargo.toml`.
- Codice, commenti, messaggi di commit e documentazione pubblica in inglese. I documenti in `docs/spikes/` e `docs/superpowers/` possono essere in italiano.
- Niente segni tipici del testo generato da AI in nessun file o commit: niente lineette lunghe (U+2014, U+2013), frecce (U+2192), carattere dei puntini di sospensione (U+2026), virgolette tipografiche (U+201C, U+201D, U+2018, U+2019). Dal Task 2 in poi, prima di ogni commit `python scripts/check_text_style.py` deve uscire con codice 0; prima del Task 2 si controlla a mano.
- Nessuna menzione di Claude come autore o co-autore in commit, PR o file.
- Nessun segreto committato (chiave AcoustID, password): si leggono da variabili d'ambiente (`ARMONIKA_ACOUSTID_KEY`) o da file in `.gitignore`.
- Nessun file musicale committato, tranne i modelli di silenzio in `bench/templates/` (pochi KB, generati e documentati).
- Multipiattaforma: tutto il codice di produzione compila e passa i test su Linux, Windows e macOS; percorsi solo tramite `std::path`, mai concatenando `/` o `\`.
- User-Agent per MusicBrainz e AcoustID: `Armonika/<versione> ( https://github.com/filocorsa/Armonika )`; massimo 1 richiesta al secondo a MusicBrainz, 3 al secondo ad AcoustID.
- Soglie di riferimento per gli spike (design §3): ricerca < 50 ms p95 su 100.000 tracce con 1 errore di battitura; prima scansione di 50.000 file in < 5 minuti su HDD.
- Termini del Glossario usati in modo esatto anche nei documenti degli spike (file, traccia, registrazione, edizione, eccetera).

## Review Focus

1. **Nomi di file e percorsi "difficili"** (accenti, caratteri CJK, spazi, caratteri vietati su Windows come `:` `?` `"`): il generatore deve produrli e nessuno strumento deve rompersi. Test in Task 4 (`generates_safe_paths_for_hostile_names`).
2. **Collisioni maiuscole/minuscole** su Windows e macOS (es. artisti "ABBA" e "Abba"): il generatore non deve sovrascrivere file. Test in Task 4 (`case_insensitive_collisions_do_not_overwrite`).
3. **Ricerca con o senza accenti** ("beyonce" deve trovare "Beyoncé"): misurato nello spike 1 (Task 6, query di tipo `diacritics`).
4. **Percorsi lunghi su Windows** (oltre 260 caratteri): il generatore deve restare sotto il limite accorciando i nomi. Test in Task 4 (`paths_stay_under_windows_limit`).
5. **Copertine incorporate grandi** (es. 5 MB): la scrittura dei tag non deve perderle né corromperle. Verificato nello spike 3 (Task 8, caso `large_cover`).

---

## Struttura dei file

```
Cargo.toml                       workspace (members: crates/*, bench/testdata; exclude: spikes/*)
rust-toolchain.toml              toolchain 1.99, componenti rustfmt e clippy
rustfmt.toml, .editorconfig      stile
.github/workflows/ci.yml         CI su Linux, Windows, macOS
crates/armonika-core/            versione e, dalla fase 1, il modello di dominio
crates/armonika-server/          eseguibile `armonika`
bench/templates/                 file audio di silenzio, uno per formato, + README con i comandi ffmpeg
bench/testdata/                  crate armonika-testdata: generatore libreria grande e libreria disordinata
bench/messy/manifest.toml        elenco dei file Creative Commons e dei danni da applicare
bench/data/                      (in .gitignore) librerie generate e file scaricati
spikes/01-search/ ... 06-decode-fingerprint/   progetti usa-e-getta
docs/spikes/01-search.md ... 06-decode-fingerprint.md   risultati e decisioni
```

---

### Task 0: Prerequisiti della macchina di sviluppo (azione dell'utente, con supporto)

**Files:**
- Create: `docs/development.md`

- [ ] **Step 1: Installare ffmpeg** (fornisce anche `ffprobe`). Su Windows: `winget install Gyan.FFmpeg`. Verifica: `ffmpeg -version` stampa la versione.
- [ ] **Step 2: Installare fpcalc** dalla release ufficiale di Chromaprint (`https://github.com/acoustid/chromaprint/releases`, archivio per il proprio sistema), mettendolo nel PATH. Verifica: `fpcalc -version` stampa la versione.
- [ ] **Step 3: Registrare l'applicazione su AcoustID** (`https://acoustid.org/new-application`, serve un account). Nome: Armonika; sito: URL della repository. Salvare la chiave nella variabile d'ambiente utente `ARMONIKA_ACOUSTID_KEY`. Verifica: `curl "https://api.acoustid.org/v2/lookup?client=$ARMONIKA_ACOUSTID_KEY&duration=1&fingerprint=AQAA"` risponde con JSON che contiene `"status"` (anche errore di fingerprint va bene, non "invalid API key").
- [ ] **Step 4: Scrivere `docs/development.md`** (in inglese): strumenti richiesti (Rust da `rust-toolchain.toml`, ffmpeg, fpcalc, Docker, Python 3 per il controllo dei simboli), variabile `ARMONIKA_ACOUSTID_KEY`, comandi per test e lint.
- [ ] **Step 5: Commit** `docs: add development setup guide`

### Task 1: Workspace ed eseguibile minimo

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `.editorconfig`
- Create: `crates/armonika-core/Cargo.toml`, `crates/armonika-core/src/lib.rs`
- Create: `crates/armonika-server/Cargo.toml`, `crates/armonika-server/src/main.rs`
- Test: `crates/armonika-server/tests/cli.rs`

**Interfaces:**
- Produces: `armonika_core::VERSION: &str` (uguale a `CARGO_PKG_VERSION`); binario `armonika` con `--version` che stampa `armonika <VERSION>`.

- [ ] **Step 1: Creare workspace e toolchain.** `Cargo.toml` con `resolver = "3"`, `members = ["crates/*", "bench/testdata"]`, `exclude = ["spikes"]`, `[workspace.package]` con `edition = "2024"`, `version = "0.0.0"`, `license = "AGPL-3.0-only"`, `repository = "https://github.com/filocorsa/Armonika"`. `[workspace.lints.rust] unsafe_code = "forbid"`, `[workspace.lints.clippy] all = "warn"`. `rust-toolchain.toml`: `channel = "1.99"`, `components = ["rustfmt", "clippy"]`.
- [ ] **Step 2: Scrivere il test che fallisce** in `crates/armonika-server/tests/cli.rs`:

```rust
#[test]
fn version_flag_prints_name_and_version() {
    let out = assert_cmd::Command::cargo_bin("armonika").unwrap()
        .arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(),
               format!("armonika {}", armonika_core::VERSION));
}
```

- [ ] **Step 3: Verificare che fallisca.** Run: `cargo test -p armonika-server`. Expected: errore di compilazione o FAIL (binario o `VERSION` mancanti).
- [ ] **Step 4: Implementare** `pub const VERSION: &str = env!("CARGO_PKG_VERSION");` in `armonika-core` e `main.rs` con clap derive (`#[command(name = "armonika", version = armonika_core::VERSION)]`).
- [ ] **Step 5: Verificare.** Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`. Expected: tutto verde.
- [ ] **Step 6: Commit** `build: set up cargo workspace and armonika binary`

### Task 2: CI su Linux, Windows e macOS

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `scripts/check_text_style.py`

**Interfaces:**
- Produces: job `test` (matrice `ubuntu-latest`, `windows-latest`, `macos-latest`) con `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; job `bench-build` (solo Linux) con `cargo bench --workspace --no-run`; job `text-style` (solo Linux) che esegue `scripts/check_text_style.py` su tutti i file tracciati. ffmpeg installato in ogni job (apt, choco, brew), perché i test di Task 4 e 5 lo usano.

- [ ] **Step 1: Scrivere `scripts/check_text_style.py`**: riceve percorsi o, senza argomenti, usa `git ls-files`; salta i file binari; esce con codice 1 stampando `file:riga` per ogni carattere tra U+2014, U+2013, U+2192, U+2026, U+201C, U+201D, U+2018, U+2019.
- [ ] **Step 2: Verificare lo script in locale.** Run: `python scripts/check_text_style.py`. Expected: codice 0 sulla repository attuale. Creare un file temporaneo con una lineetta lunga e verificare codice 1, poi cancellarlo.
- [ ] **Step 3: Scrivere `ci.yml`** con trigger `push` e `pull_request`, cache di Cargo (`Swatinem/rust-cache`), i tre job descritti in Interfaces.
- [ ] **Step 4: Commit e push, poi verificare.** Run: `gh run watch --exit-status`. Expected: tutti i job verdi sui tre sistemi.
- [ ] **Step 5: Commit** eventuali correzioni `ci: fix <motivo>`.

> Nota per l'utente (vedi domanda finale): la repository è privata, e GitHub conta i minuti di macOS 10 volte e quelli di Windows 2 volte rispetto a Linux sul piano gratuito.

### Task 3: Modelli audio di silenzio

**Files:**
- Create: `bench/templates/silence.mp3`, `.flac`, `.ogg` (Vorbis), `.opus`, `.m4a` (AAC-LC), `.wav`, `.aiff`
- Create: `bench/templates/README.md`

- [ ] **Step 1: Generare i 7 file** con ffmpeg da `anullsrc` (44100 Hz, stereo, 0,2 secondi), un comando per formato, senza tag. Per Opus usare 48000 Hz.
- [ ] **Step 2: Verificare** che ognuno sia leggibile: `ffprobe -v error -show_entries format=format_name,duration <file>` restituisce durata tra 0,15 e 0,3 secondi; dimensione totale della cartella < 200 KB.
- [ ] **Step 3: Scrivere `README.md`** con i 7 comandi esatti usati e la nota che i file sono generati, senza diritti di terzi.
- [ ] **Step 4: Commit** `test: add silent audio templates for test data generation`

### Task 4: Generatore della libreria grande

**Files:**
- Create: `bench/testdata/Cargo.toml` (crate `armonika-testdata`, `publish = false`), `bench/testdata/src/lib.rs`, `bench/testdata/src/big.rs`, `bench/testdata/src/names.rs`, `bench/testdata/src/bin/gen-big.rs`
- Test: `bench/testdata/tests/big.rs`

**Interfaces:**
- Produces:
  - `pub enum AudioFormat { Mp3, Flac, OggVorbis, Opus, M4a, Wav, Aiff }` con `fn extension(&self) -> &'static str` e `fn template_path(&self) -> PathBuf` (in `bench/templates/`).
  - `pub struct BigLibrarySpec { pub tracks: usize, pub seed: u64, pub formats: Vec<AudioFormat> }`
  - `pub struct GeneratedTrack { pub path: PathBuf, pub title: String, pub artist: String, pub album: Option<String>, pub album_artist: Option<String>, pub track: u32, pub disc: u32, pub year: u32 }` (serde Serialize/Deserialize)
  - `pub fn generate_big_library(spec: &BigLibrarySpec, out: &Path) -> anyhow::Result<Vec<GeneratedTrack>>`, che scrive anche `out/manifest.json` (l'elenco `Vec<GeneratedTrack>`, percorsi relativi a `out`).
  - Binario `gen-big --tracks <N> --seed <S> --out <DIR>`.
- Regole dei dati (fisse, per avere benchmark confrontabili): in media 10 tracce per edizione e 5 edizioni per artista; 2% di raccolte con artista dell'album "Various Artists"; 5% di edizioni su due dischi; 3% di accrediti "X feat. Y"; 10% di nomi con accenti o caratteri CJK; 1% di file senza titolo dell'album; formati assegnati a rotazione tra quelli di `spec.formats`. Percorsi secondo lo schema dei nomi del design §8.6.

- [ ] **Step 1: Scrivere i test che falliscono** in `bench/testdata/tests/big.rs`:

```rust
#[test] fn generates_requested_number_of_readable_files() {
    // 200 tracce, tutti i formati: 200 file, ognuno letto da lofty con title/artist uguali al manifest
}
#[test] fn same_seed_gives_identical_manifest() { /* due cartelle, stesso seed: manifest.json identici byte per byte */ }
#[test] fn different_seed_gives_different_manifest() { }
#[test] fn generates_safe_paths_for_hostile_names() {
    // con nomi forzati che contengono : ? " < > | * e CJK, ogni percorso generato è creabile su Windows
    // (nessun carattere vietato, nessun nome riservato come CON o NUL, nessun punto o spazio finale)
}
#[test] fn case_insensitive_collisions_do_not_overwrite() {
    // due artisti "ABBA" e "Abba" con stesso titolo: due file distinti, confronto dei percorsi in minuscolo senza duplicati
}
#[test] fn paths_stay_under_windows_limit() { /* nessun percorso relativo supera 200 caratteri */ }
#[test] fn distribution_matches_rules() {
    // con 10_000 tracce: raccolte tra 1% e 3%, feat tra 2% e 4%, senza album tra 0,5% e 1,5%
}
```

- [ ] **Step 2: Verificare che falliscano.** Run: `cargo test -p armonika-testdata`. Expected: FAIL o errore di compilazione.
- [ ] **Step 3: Implementare** `names.rs` (generatore deterministico di nomi da `rand_chacha::ChaCha8Rng` con seed, liste di parole incluse nel codice, funzione `sanitize_component(&str) -> String` che applica le regole di Windows e accorcia a 60 caratteri, e risoluzione delle collisioni con suffisso ` (2)` confrontando in minuscolo) e `big.rs` (copia del modello e scrittura dei tag con lofty).
- [ ] **Step 4: Verificare.** Run: `cargo test -p armonika-testdata`. Expected: PASS.
- [ ] **Step 5: Misurare la generazione completa.** Run: `cargo run --release -p armonika-testdata --bin gen-big -- --tracks 100000 --seed 1 --out bench/data/big-100k`. Expected: termina, `manifest.json` con 100.000 voci. Annotare durata e spazio occupato in `docs/development.md`.
- [ ] **Step 6: Commit** `test: add deterministic large library generator`

### Task 5: Libreria "disordinata apposta"

**Files:**
- Create: `bench/messy/manifest.toml`, `bench/messy/README.md`
- Create: `bench/testdata/src/messy.rs`, `bench/testdata/src/bin/gen-messy.rs`
- Test: `bench/testdata/tests/messy.rs`

**Interfaces:**
- Consumes: `AudioFormat` (Task 4).
- Produces:
  - `pub enum Damage { YoutubeTitle, StripTags, StripCover, SplitAlbum, DuplicateOtherFormat(AudioFormat), CompilationCopy, WrongCase }`
  - `pub fn apply_damage(src: &Path, dst_dir: &Path, damage: &Damage) -> anyhow::Result<Vec<PathBuf>>` (restituisce i file prodotti)
  - `pub struct GroundTruth { pub file: PathBuf, pub source_id: String, pub recording_mbid: String, pub release_mbid: String, pub is_duplicate_of: Option<String> }` scritto in `out/ground_truth.json`.
  - Binario `gen-messy --manifest bench/messy/manifest.toml --sources bench/data/cc-sources --out bench/data/messy`, che scarica i file mancanti da `source_url`, verifica `sha256` e applica i danni.
- Formato del manifest:

```toml
[[source]]
id = "nin-ghosts-01"
source_url = "https://..."
sha256 = "..."
license = "CC BY-NC-SA 3.0"
attribution = "Nine Inch Nails, Ghosts I-IV"
recording_mbid = "..."
release_mbid = "..."

[[damage]]
source = "nin-ghosts-01"
kind = "youtube_title"          # youtube_title | strip_tags | strip_cover | split_album | duplicate_other_format | compilation_copy | wrong_case
format = "mp3"                  # solo per duplicate_other_format
```

- Effetti dei danni (fissi): `youtube_title` imposta il titolo a `"<artista> - <titolo> (Official Video) [HD]"` e cancella album e numero; `strip_tags` cancella tutti i tag; `strip_cover` toglie la copertina; `split_album` scrive un titolo dell'album leggermente diverso (`"<album> (Deluxe)"`) su metà delle tracce di quell'edizione; `duplicate_other_format` crea una copia transcodificata con ffmpeg nel formato indicato (doppione: stessa registrazione e stessa edizione); `compilation_copy` crea una copia con titolo dell'album "Greatest Hits" e artista dell'album "Various Artists" (falso doppione); `wrong_case` mette titolo e artista tutto in minuscolo.

- [ ] **Step 1: Scrivere i test che falliscono** in `bench/testdata/tests/messy.rs`, uno per danno, usando i modelli di silenzio come sorgente (nessuna rete): per esempio `youtube_title_sets_video_style_title_and_clears_album`, `duplicate_other_format_produces_decodable_file_in_target_format` (verifica con `ffprobe`), `compilation_copy_is_not_marked_duplicate`, `ground_truth_lists_every_output_file`.
- [ ] **Step 2: Verificare che falliscano.** Run: `cargo test -p armonika-testdata messy`. Expected: FAIL.
- [ ] **Step 3: Implementare** `messy.rs` e il binario (download con `reqwest` bloccante, verifica `sha2`, ffmpeg chiamato come processo esterno).
- [ ] **Step 4: Verificare.** Run: `cargo test -p armonika-testdata`. Expected: PASS.
- [ ] **Step 5: Compilare il manifest reale.** Almeno 30 file sorgente da almeno 3 edizioni con licenza Creative Commons, **presenti su MusicBrainz e con impronte su AcoustID** (candidati: Nine Inch Nails "Ghosts I-IV" e "The Slip", Kevin MacLeod, Jonathan Coulton; verificare ogni registrazione su musicbrainz.org e la scheda "Fingerprints"). Ogni danno usato almeno 3 volte; almeno 5 file in Opus e 5 in M4A tra sorgenti e copie. In `README.md` la tabella di provenienza e licenza di ogni sorgente.
- [ ] **Step 6: Generare.** Run: `cargo run -p armonika-testdata --bin gen-messy -- --manifest bench/messy/manifest.toml --sources bench/data/cc-sources --out bench/data/messy`. Expected: tutti gli hash verificati, `ground_truth.json` con una voce per ogni file prodotto.
- [ ] **Step 7: Commit** `test: add messy library builder and manifest`

### Task 6: Spike 1, motore di ricerca

**Files:**
- Create: `spikes/01-search/` (progetto Cargo autonomo), `docs/spikes/01-search.md`

**Interfaces:**
- Consumes: `bench/data/big-100k/manifest.json` (Task 4).

- [ ] **Step 1: Costruire le query.** Da manifest, con seed fisso: 200 bersagli (titoli, artisti, album). Per ognuno quattro tipi di query: `prefix` (ogni prefisso da 3 caratteri alla parola intera, simulando la digitazione), `typo` (una sostituzione, inserzione o cancellazione), `diacritics` (accenti tolti, es. "beyonce"), `multiword` ("artista titolo").
- [ ] **Step 2: Implementare la variante FTS5** con `rusqlite` (feature `bundled`): tabella FTS5 con tokenizer `trigram` e una con `unicode61 remove_diacritics 2`; scegliere per ogni tipo di query la combinazione migliore, documentandola.
- [ ] **Step 3: Implementare la variante Tantivy**: indice su titolo, artista, album con normalizzazione degli accenti; `FuzzyTermQuery` con distanza 1 e prefisso.
- [ ] **Step 4: Misurare in release** per entrambe: latenza p50 e p95 per tipo di query (lato motore, indice già aperto), recall@10 (il bersaglio è tra i primi 10), tempo di costruzione dell'indice, dimensione su disco, RAM a riposo.
- [ ] **Step 5: Scrivere `docs/spikes/01-search.md`** con la tabella dei numeri e la decisione con questa regola: si sceglie FTS5 se p95 < 50 ms per tutti i tipi e recall@10 >= 0,9 per `typo` e `diacritics`; altrimenti Tantivy, se le rispetta; se nessuna le rispetta, si scrive cosa manca e si porta la questione all'utente.
- [ ] **Step 6: Commit** `spike: compare FTS5 and Tantivy for search`

### Task 7: Spike 2, MeTube

**Files:**
- Create: `spikes/02-metube/docker-compose.yml`, `spikes/02-metube/` (progetto Cargo autonomo), `docs/spikes/02-metube.md`

- [ ] **Step 1: Avviare MeTube** con l'immagine ufficiale (`ghcr.io/alexta69/metube`) e una cartella di download montata.
- [ ] **Step 2: Scegliere 10 video con musica a licenza Creative Commons** (licenza indicata sulla pagina del video), di cui almeno 2 con titoli con accenti o caratteri non latini.
- [ ] **Step 3: Implementare un client minimo** che: aggiunge un download solo audio (miglior qualità, nessuna conversione), segue lo stato fino a completamento o errore, recupera il file finito via HTTP dal server MeTube, ne verifica l'integrità con `ffprobe`.
- [ ] **Step 4: Eseguire** i 10 download due volte, più tre casi di errore: link non valido, video rimosso, MeTube spento durante un download.
- [ ] **Step 5: Scrivere `docs/spikes/02-metube.md`**: versione di MeTube provata; endpoint e payload esatti usati per aggiungere, leggere lo stato e scaricare; come si autentica (se si può); formato e codec dei file ottenuti (quanti Opus, quanti M4A); come appaiono gli errori; esito dei 20 download e del recupero via HTTP con nomi difficili. Uscita: recupero via HTTP affidabile sì o no; se no, cosa serve (es. cartella condivisa obbligatoria) e domanda all'utente.
- [ ] **Step 6: Commit** `spike: verify MeTube API and file retrieval`

### Task 8: Spike 3, scrittura dei tag con lofty

**Files:**
- Create: `spikes/03-tags/`, `docs/spikes/03-tags.md`

- [ ] **Step 1: Preparare i casi** per ognuno dei 7 formati partendo dai modelli: `basic` (titolo, artista, album, artista dell'album, traccia, disco, anno), `mbids` (identificativi MusicBrainz di registrazione, edizione, album, artisti), `multi_artist` (tag multi-valore `ARTISTS`), `unknown_tag` (un tag personalizzato scritto prima con ffmpeg), `cover` (copertina JPEG 300 KB), `large_cover` (copertina 5 MB), `unicode` (accenti e CJK).
- [ ] **Step 2: Per ogni caso**: scrivere con lofty, rileggere con lofty, rileggere con `ffprobe -show_format`, decodificare con `ffmpeg -v error -i <file> -f null -`. Poi modificare solo il titolo e verificare che tutto il resto (incluso `unknown_tag` e le copertine) sia invariato.
- [ ] **Step 3: Scrivere `docs/spikes/03-tags.md`**: tabella formato per caso (ok, perso, corrotto), nomi esatti dei tag usati per formato (ID3 frame, chiavi Vorbis comment, atomi MP4) per i campi del design, limiti trovati. Uscita: elenco dei formati che si possono scrivere in sicurezza; quelli non sicuri vengono proposti all'utente come "sola lettura" per quel formato.
- [ ] **Step 4: Commit** `spike: verify tag writing with lofty on all formats`

### Task 9: Spike 6, decodifica e impronta Chromaprint

Viene prima dello spike 4 e dello spike 5 perché entrambi ne usano il risultato.

**Files:**
- Create: `spikes/06-decode-fingerprint/`, `docs/spikes/06-decode-fingerprint.md`

**Interfaces:**
- Consumes: `bench/data/messy/` e `ground_truth.json` (Task 5).

- [ ] **Step 1: Decodifica.** Per ogni file della libreria disordinata: Symphonia per i formati che supporta; per Opus (e HE-AAC se presente) tre varianti: `symphonia-adapter-libopus`, un decoder Rust puro (`opus-decoder` o `opus-pure`), ffmpeg come processo esterno. Registrare successo o errore per file e variante.
- [ ] **Step 2: Impronte.** Per ogni file decodificato: impronta con `rusty-chromaprint` (configurazione equivalente a quella di `fpcalc`) e impronta di riferimento con `fpcalc -raw`. Calcolare la somiglianza (percentuale di bit uguali sulle parti allineate).
- [ ] **Step 3: Verifica con AcoustID.** Per 20 file: lookup con l'impronta di `fpcalc` e con quella di `rusty-chromaprint`; stesso primo risultato (registrazione) sì o no.
- [ ] **Step 4: Compilazione multipiattaforma.** Aggiungere temporaneamente un job alla CI (o un workflow a parte, da cancellare dopo) che compila la variante libopus su Linux, Windows e macOS. Registrare esito e tempi.
- [ ] **Step 5: Scrivere `docs/spikes/06-decode-fingerprint.md`** con tabelle e decisione. Regola: si sceglie la combinazione che decodifica il 100% dei formati accettati, dà lo stesso primo risultato AcoustID di `fpcalc` in almeno 19 casi su 20, e compila sui tre sistemi; a parità, si preferisce quella senza dipendenze C.
- [ ] **Step 6: Commit** `spike: choose audio decoding and fingerprinting`

### Task 10: Spike 5, impronta del contenuto

**Files:**
- Create: `spikes/05-content-hash/`, `docs/spikes/05-content-hash.md`

- [ ] **Step 1: Implementare** l'impronta come hash (BLAKE3) dei pacchetti audio compressi letti con il demuxer di Symphonia, esclusi i tag, senza decodificare. Variante B: hash solo di alcuni pacchetti campione (primi, centrali, ultimi) per non leggere tutto il file.
- [ ] **Step 2: Verificare la stabilità** su tutti i formati, libreria disordinata compresa: stessa impronta dopo modifica di titolo, aggiunta e rimozione di copertina, rinomina e spostamento; impronta diversa tra file con audio diverso e tra l'originale e il suo `duplicate_other_format`.
- [ ] **Step 3: Misurare il costo** su file reali (libreria disordinata), in ms per file e MB letti per file, per le due varianti. Estrapolare il costo su 50.000 file reali da 8 MB su HDD (circa 150 MB/s) e confrontarlo con la soglia di 5 minuti della prima scansione.
- [ ] **Step 4: Scrivere `docs/spikes/05-content-hash.md`**: tabella di stabilità, costi, e decisione su **quando** calcolarla: durante la scansione (solo se la variante scelta sta nel budget) oppure in background dopo la scansione, come l'impronta Chromaprint (design §8.7 punto 5). Se serve leggere l'intero file, la risposta attesa è "in background".
- [ ] **Step 5: Commit** `spike: evaluate content fingerprint stability and cost`

### Task 11: Spike 4, riconoscimento e punteggio di confidenza

**Files:**
- Create: `spikes/04-recognition/`, `docs/spikes/04-recognition.md`

**Interfaces:**
- Consumes: decodifica e impronta scelte nel Task 9; `ground_truth.json` (Task 5); `ARMONIKA_ACOUSTID_KEY`.

- [ ] **Step 1: Raccogliere i candidati** per ogni file della libreria disordinata: lookup AcoustID (con `meta=recordings releases releasegroups`), poi per le prime registrazioni i dettagli da MusicBrainz (edizioni in cui compaiono, durata). Salvare tutte le risposte in una cache su disco per non ripetere le richieste; rispettare i limiti di frequenza.
- [ ] **Step 2: Calcolare gli indizi** per ogni coppia file e candidato: score AcoustID, differenza di durata in secondi, somiglianza di titolo e di artista con i tag attuali (Jaro-Winkler su testo normalizzato), e per i gruppi frazione di file che corrispondono alle tracce dell'edizione e uguaglianza del numero di tracce.
- [ ] **Step 3: Cercare pesi e soglie** su griglia, misurando con la ground truth: registrazione giusta al primo posto (%), edizione giusta al primo posto (%), e per la soglia scelta la percentuale di sbagliati sopra soglia (obiettivo: 0 su questo campione) e di giusti sotto soglia.
- [ ] **Step 4: Scrivere `docs/spikes/04-recognition.md`**: numeri per tipo di danno (in particolare `youtube_title` e `strip_tags`), formula del punteggio da 0 a 100 con i pesi trovati, soglia di "corrispondenza incerta" e soglia per "conferma tutti quelli sopra la soglia", casi falliti con spiegazione. Avvertenza esplicita: il campione è piccolo e i pesi vanno ricontrollati in fase 2.
- [ ] **Step 5: Commit** `spike: measure recognition and calibrate confidence score`

### Task 12: Chiusura della fase 0

**Files:**
- Modify: `docs/superpowers/specs/2026-11-12-armonika-design.md` (§7.2, §7.3, §8.4, §8.7 punto 5, §11)
- Create: `docs/spikes/README.md`

- [ ] **Step 1: Riportare le decisioni nel design**, segnandole come esito degli spike con data: motore di ricerca (§7.3), decodifica e impronta Chromaprint (§7.2), endpoint MeTube e recupero file (§8.4), formula e soglie di confidenza (§8.2), quando si calcola l'impronta del contenuto (§8.1 e §8.7), formati scrivibili in sicurezza (§7.2).
- [ ] **Step 2: Scrivere `docs/spikes/README.md`**: indice degli spike con una riga di esito ciascuno.
- [ ] **Step 3: Verificare i criteri di uscita della fase 0** (design §9): motore di ricerca scelto con dati; recupero file da MeTube verificato; CI verde sui tre sistemi (`gh run list --limit 1` con stato `success`).
- [ ] **Step 4: Controllo dei simboli.** Run: `python scripts/check_text_style.py`. Expected: codice 0.
- [ ] **Step 5: Commit e push** `docs: record phase 0 spike outcomes in design`
- [ ] **Step 6: Presentare all'utente** il riepilogo degli esiti e le eventuali domande emerse dagli spike, prima di pianificare la fase 1.
