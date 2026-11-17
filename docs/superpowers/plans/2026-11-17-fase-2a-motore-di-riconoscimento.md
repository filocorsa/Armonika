# Fase 2, blocco A: motore di riconoscimento e taratura. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** sapere, con numeri misurati sui file veri dell'utente, quante tracce Armonika sistemerebbe da sola: un motore di riconoscimento a due punteggi (registrazione; album ed edizione), un comando di prova a vuoto che non scrive nei file, e strumenti che tarano pesi e soglie in un file di dati versionato.

**Architecture:** una crate nuova, `armonika-recognition`, con due parti separate: `engine` (puro: niente rete, niente database, niente file audio; riceve tag e candidati, restituisce punteggi, decisioni e motivi) e `lookup` (client AcoustID e MusicBrainz con limiti di richieste e cache nel database, raccolta dei candidati). L'impronta Chromaprint di un file sta in `armonika-media::fingerprint`; il calcolo in background sta nello scanner accanto all'hasher, con la stessa pausa durante le scansioni. Il comando `armonika recognize --dry-run` unisce le parti e scrive rapporto e pacchetto di taratura; i due strumenti in `bench/recognition-tools` (giudizio e taratura) leggono solo il pacchetto e chiamano `engine`, così il punteggio esiste in un posto solo.

**Tech Stack:** come la fase 1 (Rust 1.99, edition 2024, rusqlite, ureq 3, serde, clap, tracing, lofty 0.25 in sola lettura, symphonia 0.6.1), più: `rusty-chromaprint` 0.3.0, `opus-pure` 0.2.3, `base64` 0.23 (già nel core), `strsim` (Jaro-Winkler), `toml`, `rand` + `rand_chacha` (già nel workspace), `ratatui` + `crossterm` (programmino di giudizio), `webbrowser` (tasto `m`). Test: `wiremock` (già usato da `armonika-scrobble`), `tempfile`, `assert_cmd`.

**Spec:** `docs/superpowers/specs/2026-11-17-fase-2a-motore-di-riconoscimento-design.md` (questo piano lo implementa). Valgono anche il design principale `docs/superpowers/specs/2026-11-12-armonika-design.md` (§7.2, §8.1, §8.2, §8.7 punti 5, 7, 10, 15, §11 punti 1, 7-11) e gli spike `docs/spikes/04-recognition.md`, `docs/spikes/06-decode-fingerprint.md`, `docs/spikes/07-he-aac.md`. Chi esegue legge spec e piano insieme.

## Decisioni di questo piano

Sono le scelte che lo spec lascia al builder (§13) più due casi che lo spec non copre. Ognuna sta nel task indicato.

1. **Crate `armonika-recognition`** (Task 4), non ancora `armonika-organizer`: coda, scrittura dei tag e registro arrivano nei blocchi successivi e potranno assorbirla o dipenderne. Dipende da `armonika-core` (per `text::normalize`, `artists::split_credit`, `db`), mai dal server.
2. **Impronte audio salvate per impronta del contenuto** (Task 3): tabella `audio_fingerprints` con chiave `content_hash`. Due copie con lo stesso audio condividono la riga; un file che cambia audio cambia `content_hash` e quindi riceve un'impronta nuova, senza colonne nuove su `tracks`. Il calcolo parte solo dopo l'impronta del contenuto (`hash_state = 'done'`), come chiede §3 dello spec.
3. **Durata per AcoustID dal database** (Task 2, Task 9): l'impronta decodifica solo i primi 120 s; la durata è `tracks.duration_ms` (lofty). Così non si decodifica tutto il file.
4. **Cache delle risposte nel database** (Task 8): tabella `lookup_cache`, chiave = servizio + richiesta canonica **senza** la chiave AcoustID, risposte 404 comprese. Nessuna scadenza in questo blocco: un secondo giro deve fare zero richieste e la taratura deve essere riproducibile. La scadenza si decide con la coda (blocco successivo).
5. **Chiave AcoustID solo da `ARMONIKA_ACOUSTID_KEY` o da file** (`[recognition] acoustid_key_file`, Task 10). In questo blocco nessuna chiave è incorporata nel binario; se incorporare la chiave registrata dal progetto si decide prima del blocco della coda (l'utente la registra nel Task 16).
6. **ISRC**: si chiede `/isrc/<codice>` **solo** quando AcoustID non dà candidati (come lo spec). I = 1 se il candidato ha l'ISRC del file tra i suoi; I = 0 se un **altro** candidato dello stesso file lo ha; altrimenti I si esclude (MusicBrainz può non conoscere l'ISRC).
7. **Candidati da ricerca testuale**: al massimo `text_search_candidates = 5` risultati, ognuno letto con la richiesta normale della registrazione. Non hanno A, quindi la regola "AcoustID almeno 0,9 oppure ISRC concorde" li manda all'utente: la ricerca testuale da sola non applica mai niente in automatico.
8. **Formati** (Task 9, Task 11): pacchetto `package.json` (`format_version = 1`), rapporto `report.md`, giudizi `judgments.json` accanto al pacchetto. Rapporto, motivi e messaggi in inglese (§7.7 del design); cartella di uscita predefinita `<data_dir>/recognition/<nome libreria>/` (`data/` è ignorata da git).
9. **File dei pesi**: `crates/armonika-recognition/data/params.toml`, incorporato con `include_str!` e sostituibile con `--params <file>` nel comando e negli strumenti. La taratura riscrive questo file.
10. **Formule lasciate al builder** (Task 5-7): R, punteggio dell'album e dell'edizione come definiti nei task; i pesi iniziali vengono dallo spike 4 e la taratura li cerca.
11. **Separatore `/` (regola R32) solo nel riconoscimento**: la scansione e la navigazione non cambiano; il motore divide su `/` solo se il file è un MP3 con ID3v2.3 e ogni parte ha almeno 3 caratteri.
12. **Rami e PR** (modo concordato con l'utente il 2026-11-17, registro della fase 1): ogni milestone su un ramo `phase-2a/<milestone>` partito da `main`; PR verso `main` in bozza con CI verde, unite dall'utente (squash) solo dopo che `docs/phase-1-acceptance.md` è su `main` (R35). Se una milestone serve a un'altra e la PR non è ancora unita, il ramo nuovo parte dal ramo precedente; dopo lo squash si sposta con `git rebase --onto main <vecchia base> <ramo>`, come nella fase 1.

## Global Constraints

- Licenza AGPL-3.0; ogni crate nuova ha `license.workspace = true`, `edition.workspace = true`, `version.workspace = true`, `repository.workspace = true` e `[lints] workspace = true`.
- Codice, commenti, messaggi di commit, README, rapporti e documentazione pubblica (`docs/*.md`) in inglese; questo piano e i documenti in `docs/superpowers/` in italiano.
- Niente segni tipici del testo generato: lineette lunghe (U+2014, U+2013), frecce (U+2192), puntini di sospensione come carattere unico (U+2026), virgolette tipografiche (U+201C, U+201D, U+2018, U+2019). Prima di ogni commit: `python scripts/check_text_style.py` esce con codice 0.
- Nessuna menzione di Claude come autore o co-autore in commit, PR o file.
- Prima di ogni commit: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- Rami `phase-2a/<milestone>` da `main`; mai push su `main`; nessun codice di questo blocco unito a `main` prima di `docs/phase-1-acceptance.md` (R35). Durante B6 (fino a circa il 2026-11-24) il server di casa resta sull'immagine attuale.
- Nomi dei tipi dal Glossario: `Track`, `Recording`, `Release`, `ReleaseGroup`, `Library`. Una traccia non è mai chiamata `File`.
- **Nessun file di una libreria viene scritto, spostato o cancellato** in questo blocco. Lofty solo in lettura nei crate del prodotto; l'unico codice che scrive file audio è `gen-messy`, e solo nella sua cartella di uscita. Il comando di prova a vuoto scrive nel database solo `audio_fingerprints`, `audio_fingerprint_failures` e `lookup_cache`.
- Limiti di rete fissi nel codice, non configurabili: MusicBrainz al massimo 1 richiesta ogni 1,1 s, AcoustID al massimo 3 al secondo (una ogni 334 ms). User-Agent `Armonika/<versione> ( https://github.com/filocorsa/Armonika )`, dalla funzione unica `armonika_core::user_agent()` (Task 8).
- La chiave AcoustID non compare mai in log, errori, rapporti, pacchetti, cache o `Debug`.
- Dati privati della libreria dell'utente (elenchi di brani, pacchetti, giudizi, rapporti, risposte in cache) mai nella repository; nei documenti solo numeri aggregati.
- Il punteggio esiste solo in `armonika_recognition::engine`: gli strumenti in `bench/` lo chiamano, non lo ricopiano.
- Le regole di sicurezza (`[safety]` del file dei pesi) la taratura non le cerca: cambiano solo con una decisione scritta in `docs/recognition-calibration.md`.
- Multipiattaforma: percorsi con `std::path`; nel database e nel pacchetto i percorsi relativi usano `/`.

## Review Focus

1. **Prova a vuoto mentre il server è acceso sullo stesso database** (sul server di casa si lancia con `docker exec`): due processi scrittori. Il comando aspetta il proprio turno (busy timeout) e finisce; il server non vede errori. Test nel Task 10 (`dry_run_while_another_writer_holds_the_db`).
2. **Prova interrotta a metà** (rete che cade, Ctrl-C): le risposte già avute restano in cache, il pacchetto non viene scritto a metà, e il giro successivo non rifà le richieste già fatte. Test nel Task 10 (`rerun_after_failure_uses_cache`).
3. **Dati MusicBrainz incompleti o spariti**: AcoustID che indica una registrazione unita o cancellata (404), registrazione senza durata, edizione senza data o senza tipo dell'album, accredito vuoto. Niente panico: il 404 toglie il candidato con il motivo, D si esclude, l'edizione senza data va in fondo agli spareggi. Test nel Task 8 (`recording_404_is_none_and_cached`, `parses_recording_without_length_and_dates`) e nel Task 6 (`missing_length_excludes_duration`).
4. **File che non si decodifica o cambiato dopo la scansione** (HE-AAC senza ffmpeg, file troncato, file sostituito): la traccia riceve il motivo "no audio fingerprint: ..." e si riconosce dai tag (ISRC, ricerca testuale); il giro continua. Test nel Task 3 (`undecodable_file_is_recorded_as_failure`) e nel Task 10 (`track_without_fingerprint_still_gets_a_decision`).
5. **Nomi con `&`, virgola, "and", e testo non latino**: "Simon & Garfunkel" ed "Earth, Wind & Fire" restano un artista; titoli in giapponese o cirillico si normalizzano e si confrontano senza perdere caratteri. Test nel Task 5 (`never_splits_on_ampersand_comma_and`) e nel Task 4 (`cleanup_keeps_non_latin_titles`).

---

## Struttura dei file

```
crates/armonika-core/
  migrations/0005_isrc_id3.sql          colonne isrcs, id3v2_major su tracks
  migrations/0006_audio_fingerprints.sql audio_fingerprints, audio_fingerprint_failures
  migrations/0007_lookup_cache.sql      lookup_cache
  src/fingerprints.rs                   lettura e scrittura delle impronte audio
  src/recognition_rows.rs               tracce di una libreria con i campi per il riconoscimento
  src/lib.rs                            + user_agent()
crates/armonika-media/
  src/probe.rs                          + ISRC, versione ID3v2 (TAG_SCHEMA_VERSION 2)
  src/fingerprint.rs                    decodifica (Symphonia, opus-pure, ffmpeg) e Chromaprint
crates/armonika-scanner/
  src/gate.rs                           pausa per libreria condivisa da hasher e fingerprinter
  src/fingerprinter.rs                  impronte audio in background
crates/armonika-recognition/            NUOVA
  data/params.toml                      pesi, soglie, regole di sicurezza, elenchi di testo
  src/lib.rs
  src/params.rs                         Params: lettura, validazione, scrittura
  src/model.rs                          TrackEvidence, Recording, Release, ReleaseGroup, Candidate, ...
  src/engine/mod.rs                     recognize(): gruppi, due punteggi, decisione
  src/engine/title.rs                   pulizia del titolo, forma o sostanza
  src/engine/artists.rs                 artisti del file, corrispondenza con l'accredito
  src/engine/recording.rs               indizi e punteggio della registrazione
  src/engine/album.rs                   punteggio dell'album e dell'edizione nel gruppo
  src/engine/decision.rs                Decision, Reason, Proposal
  src/lookup/mod.rs                     Lookup: client con limiti e cache
  src/lookup/acoustid.rs, musicbrainz.rs, cache.rs, limiter.rs
  src/lookup/gather.rs                  candidati di una traccia
  src/package.rs                        pacchetto di taratura e rapporto
  src/judgment.rs                       giudizi, campione stratificato, verità della libreria disordinata
  tests/fixtures/                       risposte AcoustID e MusicBrainz registrate (dati CC0)
crates/armonika-server/
  src/config.rs                         + [recognition]
  src/cli/recognize.rs                  armonika recognize --dry-run
bench/recognition-tools/                NUOVA, binari armonika-judge e armonika-calibrate
  src/bin/armonika-judge.rs, src/judge/{app.rs, ui.rs}
  src/bin/armonika-calibrate.rs, src/calibrate/{grid.rs, report.rs}
bench/testdata/src/messy.rs             API native di lofty, cartelle per edizione, danni nuovi
bench/messy/manifest.toml, README.md    sorgenti nuove
docs/recognition-calibration.md         esito della taratura (solo numeri aggregati)
```

## Milestone

| Milestone | Ramo | Task | Dipende da |
|---|---|---|---|
| A1 impronte | `phase-2a/a1-fingerprint` | 1, 2, 3 | `main` |
| A2 motore | `phase-2a/a2-engine` | 4, 5, 6, 7 | `main` |
| A3 prova a vuoto | `phase-2a/a3-dry-run` | 8, 9, 10 | A1, A2 |
| A4 strumenti | `phase-2a/a4-tools` | 11, 12 | A3 |
| A5 libreria disordinata | `phase-2a/a5-messy` | 13, 14 | `main` |
| A6 taratura con l'utente | `phase-2a/a6-calibration` | 15, 16 | tutte; Task 16 anche B6, R35 e la chiave AcoustID dell'utente |

A1, A2 e A5 sono indipendenti e possono procedere in parallelo. Alla fine di ogni milestone: revisione dell'intera milestone, poi push e PR in bozza.

---

### Task 1: ISRC e versione ID3v2 nella lettura dei tag

**Files:**
- Modify: `crates/armonika-core/src/model.rs` (`TagFacts`)
- Create: `crates/armonika-core/migrations/0005_isrc_id3.sql`
- Modify: `crates/armonika-core/src/db.rs` (`MIGRATIONS`)
- Modify: `crates/armonika-core/src/track_files.rs` (scrittura delle colonne di tag)
- Modify: `crates/armonika-media/src/probe.rs`
- Test: `crates/armonika-media/tests/probe.rs`, `crates/armonika-scanner/tests/scan.rs`

**Interfaces:**
- Produces: `TagFacts.isrcs: Vec<String>` (maiuscole, senza trattini, solo codici validi `^[A-Z]{2}[A-Z0-9]{3}[0-9]{7}$`, senza doppioni); `TagFacts.id3v2_major: Option<u8>` (3 o 4, solo per file MP3 che iniziano con un'intestazione ID3v2); colonne `tracks.isrcs TEXT` (valori uniti da U+001F, NULL se vuoto) e `tracks.id3v2_major INTEGER`; `TAG_SCHEMA_VERSION = 2`.

- [ ] **Step 1: Write the failing tests**

In `crates/armonika-media/tests/probe.rs`, con file creati da `armonika-testdata` come gli altri test del file:
```rust
#[test]
fn reads_isrc_normalized() {
    // TSRC "us-s1z-99-00001" in un mp3
    assert_eq!(facts.tags.isrcs, vec!["USS1Z9900001"]);
}
#[test]
fn drops_invalid_isrc() { /* TSRC "not an isrc" */ assert!(facts.tags.isrcs.is_empty()); }
#[test]
fn reports_id3v2_major_version() {
    // mp3 salvato con WriteOptions::default().use_id3v23(true)
    assert_eq!(v23.tags.id3v2_major, Some(3));
    assert_eq!(v24.tags.id3v2_major, Some(4));
    assert_eq!(flac.tags.id3v2_major, None);
}
```
In `crates/armonika-scanner/tests/scan.rs`:
```rust
#[test]
fn isrc_and_id3_version_are_stored() {
    // dopo la scansione: SELECT isrcs, id3v2_major FROM tracks
    assert_eq!(row, (Some("USS1Z9900001".to_string()), Some(3)));
}
#[test]
fn tracks_read_with_schema_1_are_read_again() {
    // UPDATE tracks SET tag_schema = 1; nuova scansione
    assert_eq!(report.files_read, tracks);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-media --test probe isrc id3v2 && cargo test -p armonika-scanner --test scan isrc tracks_read_with_schema_1`
Expected: FAIL (campi `isrcs` e `id3v2_major` inesistenti)

- [ ] **Step 3: Implement**

`ItemKey::Isrc` di lofty per l'ISRC (dividere su NUL, `;`, `/`, `,` e spazi, togliere i trattini, maiuscole, filtrare con la forma sopra). `id3v2_major`: per `AudioFormat::Mp3` leggere i primi 10 byte del file: se iniziano con `ID3`, il quarto byte è la versione principale; altrimenti `None`. La migrazione `0005_isrc_id3.sql` aggiunge le due colonne; `TAG_SCHEMA_VERSION` passa a 2, così la scansione successiva rilegge ogni traccia una volta (meccanismo già esistente). Le due colonne si scrivono negli stessi punti di `artists_tag` in `track_files.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/armonika-core crates/armonika-media crates/armonika-scanner
git commit -m "scanner: read ISRC and the ID3v2 version of mp3 files"
```

---

### Task 2: impronta Chromaprint di un file

**Files:**
- Create: `crates/armonika-media/src/fingerprint.rs`
- Modify: `crates/armonika-media/src/lib.rs`, `crates/armonika-media/Cargo.toml` (`rusty-chromaprint = "0.3.0"`, `opus-pure = "0.2.3"`, `base64 = "0.23.1"`)
- Test: `crates/armonika-media/tests/fingerprint.rs`

**Interfaces:**
- Produces:
  ```rust
  pub const FINGERPRINT_SECONDS: u32 = 120;
  pub enum Decoder { Symphonia, OpusPure, Ffmpeg }          // as_str(): "symphonia" | "opus-pure" | "ffmpeg"
  pub struct AudioFingerprint { pub fingerprint: String, pub decoder: Decoder }
  pub enum FingerprintError { Io{..}, Unsupported{ path, message }, Decode{ path, message }, TooShort(PathBuf) }
  pub fn fingerprint(path: &Path, ffmpeg: Option<&Path>) -> Result<AudioFingerprint, FingerprintError>
  ```
  `fingerprint` è la stringa compressa come `fpcalc` (`FingerprintCompressor` di rusty-chromaprint, poi base64 URL-safe senza padding).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_template_format_gives_a_fingerprint() {
    // bench/templates/silence.{mp3,flac,ogg,opus,m4a,wav,aiff}: Ok, fingerprint non vuota;
    // opus usa Decoder::OpusPure, gli altri Decoder::Symphonia
}
#[test]
fn same_pcm_gives_same_fingerprint() {
    // un WAV sintetico di 30 s (accordo di tre sinusoidi, scritto dal test senza dipendenze)
    // e lo stesso PCM in AIFF: impronte uguali
}
#[test]
fn m4a_edit_list_priming_is_skipped() {
    // decode_pcm (pub(crate), testato in un modulo #[cfg(test)]) sul template m4a:
    // campioni per canale == quelli del template wav +/- 1 pacchetto (1024)
}
#[test]
fn unsupported_codec_without_ffmpeg_is_unsupported() {
    // file m4a con il campo codec cambiato in un codec che Symphonia non conosce
    assert!(matches!(fingerprint(&path, None), Err(FingerprintError::Unsupported { .. })));
}
#[test]
fn truncated_file_is_an_error_not_a_panic() { /* primi 2 KiB di un flac */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-media --test fingerprint`
Expected: FAIL (modulo inesistente)

- [ ] **Step 3: Implement `fingerprint(path, ffmpeg)`**

Porta il codice di decodifica di `spikes/04-recognition/src/main.rs` (Symphonia; Opus con `OggOpusReader` e `Trim` di opus-pure) fermandosi a 120 s di audio. `Configuration::preset_test2()`, `Fingerprinter::start(rate, channels)`. M4A: leggere `media_time` dell'edit list (`elst`) dal file e saltare quei campioni per canale (mai 1024 fisso: con ALAC il valore è 0); il box si trova scorrendo `moov/trak/edts/elst` con un piccolo lettore di box, perché Symphonia 0.6.1 non lo espone. Se Symphonia rifiuta il codec e `ffmpeg` è `Some`, ripiego con `ffmpeg -v error -nostdin -protocol_whitelist file,pipe -i <file> -t 120 -map 0:a:0 -f s16le -ac 2 -ar 44100 -`. Ogni decodifica dentro `catch_unwind` come `hash_isolated` dello scanner. Meno di 3 s di audio: `TooShort`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armonika-media --test fingerprint`
Expected: PASS

- [ ] **Step 5: Controllo manuale contro lo spike (non in CI)**

Run: con `bench/data/messy` generata, un test `#[ignore]` `matches_spike_fingerprints` che confronta le impronte con `bench/data/recognition/features.json` dello spike 4, se presente.
Expected: per ogni file almeno il 98% di bit uguali (lo spike riporta minimi di 96,8% solo per M4A senza edit list; con l'edit list applicata il minimo era 98,8%).

- [ ] **Step 6: Commit**

```bash
git add crates/armonika-media Cargo.lock
git commit -m "media: Chromaprint fingerprint of the first 120 seconds of a file"
```

---

### Task 3: impronte audio in background

**Files:**
- Create: `crates/armonika-core/migrations/0006_audio_fingerprints.sql`, `crates/armonika-core/src/fingerprints.rs`
- Modify: `crates/armonika-core/src/db.rs`, `crates/armonika-core/src/lib.rs`
- Create: `crates/armonika-scanner/src/gate.rs`, `crates/armonika-scanner/src/fingerprinter.rs`
- Modify: `crates/armonika-scanner/src/hasher.rs` (usa `gate.rs`), `crates/armonika-scanner/src/controller.rs`, `crates/armonika-scanner/src/lib.rs`
- Modify: `crates/armonika-server/src/config.rs` (`[recognition]`), `crates/armonika-server/src/serve.rs`, `docs/configuration.md`
- Test: `crates/armonika-core/tests/fingerprints.rs`, `crates/armonika-scanner/tests/fingerprinter.rs`, `crates/armonika-server/tests/config.rs`

**Interfaces:**
- Consumes: `armonika_media::fingerprint::{fingerprint, AudioFingerprint, FingerprintError}` (Task 2).
- Produces:
  ```rust
  // armonika_core::fingerprints
  pub struct FingerprintJob { pub key: i64, pub library: LibraryId, pub rel_path: String, pub size: u64, pub mtime_ms: i64, pub content_hash: Vec<u8> }
  pub struct StoredFingerprint { pub fingerprint: String, pub decoder: String }
  pub enum FingerprintState { Done(StoredFingerprint), Failed(String), Pending }
  pub fn pending(conn: &Connection, after: i64, paused: &[LibraryId], limit: usize) -> Result<Vec<FingerprintJob>>
  pub fn state(conn: &Connection, content_hash: &[u8]) -> Result<FingerprintState>
  pub fn store(conn: &Connection, content_hash: &[u8], fp: &str, decoder: &str) -> Result<()>
  pub fn store_failure(conn: &Connection, content_hash: &[u8], reason: &str) -> Result<()>
  pub fn counts(conn: &Connection, library: LibraryId) -> Result<FingerprintCounts>  // done, failed, pending
  // armonika_scanner
  pub struct FingerprintOptions { pub ffmpeg: Option<PathBuf> }
  ScanController::new(db, opts, changes, fingerprint: Option<FingerprintOptions>)  // None: spento
  pub fn fingerprint_pending(db: &Db, opts: &FingerprintOptions, gate: &WorkGate, stop: &CancellationToken) -> FingerprintReport
  ```
  Tabelle: `audio_fingerprints(content_hash BLOB PRIMARY KEY, fingerprint TEXT NOT NULL, decoder TEXT NOT NULL, computed_at INTEGER NOT NULL)`, `audio_fingerprint_failures(content_hash BLOB PRIMARY KEY, reason TEXT NOT NULL, failed_at INTEGER NOT NULL)`. Configurazione: `[recognition] background_fingerprint = true` (default), e il `ffmpeg_path` esistente come ripiego.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/armonika-core/tests/fingerprints.rs
#[test] fn pending_lists_only_present_hashed_tracks_without_fingerprint() { /* hash_state pending o missing: esclusi */ }
#[test] fn copies_with_the_same_audio_share_one_fingerprint() { /* due tracce, stesso content_hash: store una volta, counts.done == 2 */ }
#[test] fn failure_is_not_pending_any_more() {}
// crates/armonika-scanner/tests/fingerprinter.rs
#[test] fn fingerprints_after_the_content_hash() { /* scan, hash_pending, fingerprint_pending: counts.done == tracce */ }
#[test] fn undecodable_file_is_recorded_as_failure() { /* file .m4a con codec sconosciuto, ffmpeg None: counts.failed == 1, reason non vuota */ }
#[test] fn file_changed_since_the_scan_is_skipped() { /* mtime cambiato: né done né failed */ }
#[test] fn scan_pauses_the_fingerprinter() { /* come pause_waits_for_the_track_in_flight dell'hasher */ }
#[test] fn disabled_fingerprinter_does_nothing() { /* ScanController::new(.., None) */ }
// crates/armonika-server/tests/config.rs
#[test] fn recognition_section_defaults_and_env() { /* background_fingerprint true; ARMONIKA_RECOGNITION__BACKGROUND_FINGERPRINT=false */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-core --test fingerprints && cargo test -p armonika-scanner --test fingerprinter && cargo test -p armonika-server --test config recognition`
Expected: FAIL

- [ ] **Step 3: Implement**

`gate.rs` estrae da `hasher.rs` la pausa per libreria (`paused`, `busy`, `Condvar`, guardia `Busy`) in `pub struct WorkGate` con `pause`, `resume`, `is_paused`, `enter`; hasher e fingerprinter ne hanno uno ciascuno e il controller mette in pausa entrambi prima di scansionare una libreria (prima l'hasher, poi il fingerprinter, ripresa in ordine inverso). Il fingerprinter gira su un thread `armonika-fingerprint` a priorità minima (`lower_priority()` esistente), lavora 20 tracce alla volta, ricontrolla dimensione e data del file prima e dopo il calcolo (come `still_as_scanned`), e scrive successo o fallimento in una transazione breve. Non emette `LibraryChange`. In `config.rs` la sezione `recognition` entra in `TOP_LEVEL_KEYS`. In `serve.rs` passa `Some(FingerprintOptions { ffmpeg: config.ffmpeg_path.clone() })` solo se `config.recognition.background_fingerprint`. Documenta la chiave in `docs/configuration.md`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates docs/configuration.md
git commit -m "scanner: compute audio fingerprints in the background after the content hash"
```

- [ ] **Step 6: Fine milestone A1**

Revisione dell'intera milestone; push di `phase-2a/a1-fingerprint` e PR in bozza. Il job `bench-smoke` della CI misura con il lavoro in background attivo (`--quiet-timeout-secs 0`): deve restare verde, ed è la verifica del vincolo "non peggiorare le soglie della fase 1".

---

### Task 4: crate `armonika-recognition`, modello, parametri e pulizia del titolo

**Files:**
- Create: `crates/armonika-recognition/Cargo.toml`, `src/lib.rs`, `src/model.rs`, `src/params.rs`, `src/engine/mod.rs`, `src/engine/title.rs`, `data/params.toml`
- Test: `crates/armonika-recognition/tests/params.rs`, `crates/armonika-recognition/tests/title.rs`

**Interfaces:**
- Produces (tutti `Serialize + Deserialize + Clone + Debug + PartialEq`):
  ```rust
  // model.rs
  pub struct TrackEvidence {
      pub track: String, pub rel_path: String, pub group: String,
      pub title: Option<String>, pub artist_credit: Option<String>, pub artists: Vec<String>,
      pub mb_artist_ids: Vec<String>, pub album_title: Option<String>,
      pub album_artist_credit: Option<String>, pub track_total: Option<u32>,
      pub compilation: bool, pub isrcs: Vec<String>, pub id3v2_major: Option<u8>,
      pub duration_ms: Option<u64>, pub mb_release_id: Option<String>,
  }
  pub struct CreditedArtist { pub mbid: String, pub name: String, pub credited_name: String,
      pub join_phrase: String, pub sort_name: String, pub aliases: Vec<String> }
  pub struct ReleaseGroup { pub mbid: String, pub title: String,
      pub primary_type: Option<String>, pub secondary_types: Vec<String> }
  pub struct TrackPosition { pub disc: u32, pub number: String, pub position: u32 }
  pub struct Release { pub mbid: String, pub title: String, pub status: Option<String>,
      pub date: Option<PartialDate>, pub release_group: ReleaseGroup, pub track_count: Option<u32>,
      pub formats: Vec<String>, pub artist_credit: Vec<CreditedArtist>, pub position: Option<TrackPosition> }
  pub struct Recording { pub mbid: String, pub title: String, pub length_ms: Option<u64>,
      pub credit: Vec<CreditedArtist>, pub isrcs: Vec<String>, pub releases: Vec<Release> }
  pub enum CandidateSource { AcoustId, Isrc, TextSearch }
  pub struct Candidate { pub recording: Recording, pub acoustid_score: Option<f64>, pub source: CandidateSource }
  pub enum FingerprintStatus { Present, Missing(String) }
  pub struct TrackInput { pub evidence: TrackEvidence, pub fingerprint: FingerprintStatus, pub candidates: Vec<Candidate> }
  pub struct LibraryContext { pub releases: BTreeMap<String, Vec<String>> }   // release mbid -> tracce che lo hanno nei tag
  pub fn group_key(rel_path: &str, album_title: Option<&str>, album_artist: Option<&str>, track: &str) -> String
  // params.rs
  pub struct Params { pub format: u32, pub revision: u32, pub recording: RecordingParams,
      pub album: AlbumParams, pub edition: EditionParams, pub safety: SafetyParams, pub text: TextParams }
  impl Params { pub fn embedded() -> Params; pub fn from_toml(s: &str) -> Result<Params, ParamsError>;
      pub fn load(path: &Path) -> Result<Params, ParamsError>; pub fn to_toml(&self) -> String }
  // engine/title.rs
  pub struct CleanTitle { pub norm: String, pub featured: Vec<String> }
  pub fn clean_file_title(text: &TextParams, title: &str, file_artists: &[String]) -> CleanTitle
  pub fn version_markers(text: &TextParams, norm_title: &str) -> BTreeSet<String>
  ```
  `group_key`: cartella di `rel_path` + `text::normalize` del titolo dell'album + dell'artista dell'album; senza titolo dell'album, `"track:" + track` (gruppo da solo). `PartialDate` è quello di `armonika_core::model` (aggiungere lì `Serialize, Deserialize`: il core dipende già da `serde`).

`data/params.toml` iniziale (pesi dello spike 4 più I; soglie di partenza, da tarare):
```toml
format = 1
revision = 1
[recording]
threshold = 90.0
duration_scale_s = 10.0
title_form_min = 0.95
weights = { acoustid = 1.0, duration = 2.0, title = 1.0, artists = 1.0, isrc = 2.0 }
[album]
threshold = 90.0
weights = { original = 1.0, title = 1.0, uniqueness = 1.0 }
[edition]
threshold = 90.0
weights = { coverage = 1.0, track_count = 1.0, in_library = 1.0, official = 1.0 }
[safety]            # not searched by the calibration
acoustid_min_candidate = 0.5
acoustid_min_auto = 0.9
require_title_or_artist = true
text_search_candidates = 5
[text]
decorations = ["official", "official video", "official audio", "official music video", "video", "audio", "lyric", "lyrics", "lyric video", "hd", "hq", "4k", "visualizer", "visualiser", "mv"]
store_suffixes = ["remastered", "remaster", "remastered {year}", "{year} remaster", "{year} remastered", "remastered version", "{year} remastered version"]
feat_tokens = ["feat", "ft", "featuring"]
bracket_feat_tokens = ["con", "with"]
version_markers = ["instrumental", "live", "acoustic", "remix", "demo", "karaoke", "a cappella", "acapella", "radio edit", "extended", "unplugged"]
various_artists = ["various artists", "various", "artisti vari", "va"]
```

- [ ] **Step 1: Write the failing tests**

```rust
// tests/params.rs
#[test] fn embedded_params_load_and_validate() {}
#[test] fn rejects_negative_weight_threshold_over_100_and_unknown_format() {}
#[test] fn toml_round_trip_is_lossless() { assert_eq!(Params::from_toml(&p.to_toml()).unwrap(), p); }
// tests/title.rs: una tabella (titolo del file, artisti del file, titolo pulito atteso, feat attesi)
#[test] fn every_listed_suffix_and_decoration_has_a_case() {
    // per ogni voce di decorations, store_suffixes ({year} = 2011), feat_tokens,
    // bracket_feat_tokens e version_markers del file embedded esiste almeno una riga
    // della tabella che la usa: il test fallisce se si aggiunge una voce senza caso
}
#[test] fn cleanup_cases() {
    // "Artist - Song (Official Video) [HD]" con artista "Artist" -> "song"
    // "Song - Remastered 2011" -> "song"; "Song (2011 Remaster)" -> "song"
    // "Song feat. B" -> "song", featured ["B"]; "Song (con Andrea)" -> "song", featured ["Andrea"]
    // "Con te partiro" -> "con te partiro" (con fuori parentesi resta)
    // "Song (Live)" -> "song live" (i segni di versione non si tolgono mai)
}
#[test] fn prefix_removed_only_when_it_is_a_file_artist() { /* "Other - Song" con artista "Artist" resta */ }
#[test] fn cleanup_keeps_non_latin_titles() { /* "夜に駆ける" e "Кино - Группа крови" con artista "Кино" */ }
#[test] fn version_markers_found_on_both_sides() { /* "song instrumental" -> {"instrumental"} */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition`
Expected: FAIL (crate vuota)

- [ ] **Step 3: Implement**

Crate nel workspace (`members` include già `crates/*`); dipendenze `armonika-core`, `serde`, `toml`, `strsim`, `thiserror`. La pulizia lavora su testo normalizzato con `armonika_core::text::normalize`, salvo l'estrazione dei feat che usa il testo originale per conservare i nomi. Regole: le parentesi tonde o quadre il cui contenuto normalizzato è una voce di `decorations` si tolgono; una parte finale (dopo ` - ` o tra parentesi) che corrisponde a una voce di `store_suffixes` si toglie (`{year}` = quattro cifre); `feat_tokens` valgono ovunque come parola intera, `bracket_feat_tokens` solo all'inizio di una parentesi; il prefisso `X - ` si toglie solo se `normalize(X)` è uno degli artisti del file. `version_markers` non si toglie mai.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armonika-recognition`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/armonika-recognition crates/armonika-core Cargo.lock
git commit -m "recognition: new crate with the data model, the weights file and title cleanup"
```

---

### Task 5: artisti del file e corrispondenza con l'accredito

**Files:**
- Create: `crates/armonika-recognition/src/engine/artists.rs`, `src/engine/decision.rs`
- Test: `crates/armonika-recognition/tests/artists.rs`

**Interfaces:**
- Consumes: `armonika_core::artists::split_credit` (regole sicure di §8.1, già usate dalla scansione), `CleanTitle.featured` (Task 4).
- Produces:
  ```rust
  pub struct FileArtist { pub name: String, pub mbid: Option<String> }
  pub fn file_artists(ev: &TrackEvidence, featured: &[String]) -> Vec<FileArtist>
  pub enum ArtistVerdict { Form, Substance(Vec<Reason>) }
  pub struct ArtistMatch { pub value: Option<f64>, pub verdict: ArtistVerdict }
  pub fn match_artists(ev: &TrackEvidence, file: &[FileArtist], credit: &[CreditedArtist]) -> ArtistMatch
  ```
  `Reason` è definito in `engine/decision.rs` in questo task con le varianti degli artisti (`FileArtistNotInCredit(String)`, `CreditArtistNotInFile(String)`); il Task 6 aggiunge le altre.

Regole (spec §5.2): artisti del file = `split_credit(credit, artists, mb_artist_ids)`; ma se `id3v2_major == Some(3)`, il credito contiene `/`, `artists` è vuoto e ogni parte divisa su `/` ha almeno 3 caratteri dopo il trim, le parti diventano la lista `artists_tag` passata a `split_credit`; poi si aggiungono i feat tolti dal titolo; doppioni tolti per nome normalizzato. Un nome del file corrisponde a un artista dell'accredito se l'mbid coincide oppure il nome normalizzato coincide con `credited_name`, `name`, `sort_name` o un alias. Esito:
- se il credito del file normalizzato è uguale alla concatenazione normalizzata di `credited_name` e `join_phrase` dell'accredito: `Form`, `value = 1`;
- altrimenti ogni artista del file deve corrispondere; un artista della proposta senza corrispondenza è ammesso solo se il file non ha artisti o se il suo nome normalizzato compare come parola intera nel titolo o nell'artista dell'album del file; ogni violazione è un `Reason` di sostanza;
- `value`: `None` se il file non ha artisti; 1 se `Form`; con sostanza `artisti del file trovati / (artisti del file + artisti della proposta non spiegati)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn never_splits_on_ampersand_comma_and() {
    // "Simon & Garfunkel", "Earth, Wind & Fire", "Tom and Jerry", "Takagi e Ketra": un artista ciascuno
}
#[test] fn splits_slash_only_in_id3v23_with_long_parts() {
    // "Takagi/Ketra/Giusy" id3v2_major 3 -> 3 artisti; stesso testo id3v2_major 4 -> 1;
    // "AC/DC" id3v2_major 3 -> 1 (parte "DC" di 2 caratteri)
}
#[test] fn featured_from_title_become_file_artists() {}
#[test] fn single_string_credit_equal_to_one_mb_artist_is_form() { /* "Takagi & Ketra" */ }
#[test] fn joined_credit_equal_to_mb_credit_is_form() { /* file "A feat. B" vs MB [A " feat. ", B] */ }
#[test] fn file_artist_missing_from_credit_is_substance() {}
#[test] fn extra_credit_artist_allowed_only_if_named_elsewhere() {
    // MB [A, B]; file artista "A", titolo "Song (feat. B)" -> Form; titolo "Song" -> Substance
}
#[test] fn extra_credit_artist_allowed_when_file_has_no_artist() { /* value None, verdict Form */ }
#[test] fn matches_by_alias_and_mbid() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition --test artists`
Expected: FAIL

- [ ] **Step 3: Implement `file_artists` e `match_artists`** con le regole sopra.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armonika-recognition`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/armonika-recognition
git commit -m "recognition: file artists and form or substance against the MusicBrainz credit"
```

---

### Task 6: punteggio della registrazione

**Files:**
- Create: `crates/armonika-recognition/src/engine/recording.rs`
- Modify: `crates/armonika-recognition/src/engine/decision.rs` (`Reason` completo)
- Test: `crates/armonika-recognition/tests/recording.rs`

**Interfaces:**
- Consumes: Task 4 (`clean_file_title`, `version_markers`, `Params`), Task 5 (`file_artists`, `match_artists`).
- Produces:
  ```rust
  pub struct RecordingClues { pub acoustid: Option<f64>, pub duration: Option<f64>,
      pub title: Option<f64>, pub artists: Option<f64>, pub isrc: Option<f64> }
  pub struct RecordingScore { pub recording: String, pub clues: RecordingClues, pub score: f64,
      pub substance: Vec<Reason>, pub notes: Vec<Reason>, pub auto_allowed: bool }
  pub fn score_recording(p: &Params, input: &TrackInput, candidate: &Candidate) -> RecordingScore
  pub fn rank_recordings(p: &Params, input: &TrackInput) -> Vec<RecordingScore>   // migliore per primo
  pub enum Reason {   // ognuno con fn code(&self) -> &'static str e fn text(&self) -> String stabili, in inglese
      TitleEqualAfterCleanup, TitleFormDifference, TitleSubstance { file: String, proposal: String },
      VersionMarkerMismatch(String), FileArtistNotInCredit(String), CreditArtistNotInFile(String),
      IsrcAgrees, IsrcPointsElsewhere, AcoustIdBelowAutoMin(f64), TextSearchOnly,
      NoTitleNoArtist, NoFingerprint(String), DurationUnknown, NoCandidates,
      RecordingBelowThreshold(f64), PossibleCompilation, AlbumBelowThreshold(f64),
      EditionBelowThreshold(f64), NotOnGroupAlbum, }
  ```

Indizi (spec §4.1): A = `acoustid_score` (candidati sotto `acoustid_min_candidate` non arrivano qui; `None` per ISRC e ricerca testuale); D = `max(0, 1 - |durata del file - length_ms| / duration_scale_s)`, escluso se manca una delle due durate; T = Jaro-Winkler (`strsim::jaro_winkler`) tra titolo del file pulito e titolo MusicBrainz normalizzato, escluso senza titolo; R = `ArtistMatch.value`; I come nella Decisione 6. `score = 100 * somma(peso * valore) / somma(pesi degli indizi presenti)`, 0 se nessun indizio. Titolo: uguali dopo la pulizia o Jaro-Winkler almeno `title_form_min` = forma, altrimenti `TitleSubstance`; segni di versione diversi tra i due lati = `VersionMarkerMismatch`. `auto_allowed` è falso se A < `acoustid_min_auto` (o assente) e I non è 1, oppure se `require_title_or_artist` e il file non ha né titolo né artista. Ordine: punteggio, poi A, poi mbid.

- [ ] **Step 1: Write the failing tests** (candidati costruiti a mano nel test, valori dello spike 4)

```rust
#[test] fn duration_separates_ghosts_22_from_26() { /* file 151 s, A uguale: 22 (151,2 s) prima di 26 (146,1 s) */ }
#[test] fn missing_length_excludes_duration() { /* clues.duration None, score calcolato senza il peso 2 */ }
#[test] fn instrumental_version_is_substance() { /* file "The Slip", MB "The Slip (instrumental)" */ }
#[test] fn youtube_title_cleans_to_form() {}
#[test] fn isrc_agrees_and_points_elsewhere() { /* I = 1 per il candidato con l'ISRC, 0 per l'altro, None se nessuno */ }
#[test] fn low_acoustid_blocks_auto_unless_isrc_agrees() { /* A 0,85: false; A 0,85 + ISRC: true */ }
#[test] fn text_search_candidate_never_auto() {}
#[test] fn no_title_no_artist_never_auto() {}
#[test] fn reason_codes_are_unique_and_texts_ascii() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition --test recording`
Expected: FAIL

- [ ] **Step 3: Implement `score_recording` e `rank_recordings`.**

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p armonika-recognition`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/armonika-recognition
git commit -m "recognition: recording confidence with duration, title, artists and ISRC clues"
```

---

### Task 7: album, edizione, gruppi e decisione

**Files:**
- Create: `crates/armonika-recognition/src/engine/album.rs`
- Modify: `crates/armonika-recognition/src/engine/mod.rs`, `src/engine/decision.rs`
- Test: `crates/armonika-recognition/tests/decision.rs`

**Interfaces:**
- Consumes: Task 6 (`rank_recordings`, `RecordingScore`, `Reason`).
- Produces:
  ```rust
  pub struct AlbumClues { pub original: f64, pub title: Option<f64>, pub uniqueness: f64 }
  pub struct AlbumScore { pub release_group: String, pub clues: AlbumClues, pub score: f64, pub gated: Vec<Reason> }
  pub struct EditionClues { pub coverage: f64, pub track_count: Option<f64>, pub in_library: f64, pub official: f64 }
  pub struct EditionScore { pub release: String, pub clues: EditionClues, pub score: f64 }
  pub enum Decision { ApplyAll, ApplyEditionUnsure, ApplyAlbumUnsure, ToUser, NotRecognized }
  pub struct Proposal { pub title: String, pub artists: Vec<CreditedArtist>, pub recording: String,
      pub album_title: Option<String>, pub album_artists: Vec<CreditedArtist>, pub release_group: Option<String>,
      pub original_year: Option<i32>, pub release: Option<String>, pub date: Option<PartialDate>,
      pub position: Option<TrackPosition> }
  pub struct TrackDecision { pub track: String, pub decision: Decision, pub reasons: Vec<Reason>,
      pub recordings: Vec<RecordingScore>, pub album: Option<AlbumScore>, pub edition: Option<EditionScore>,
      pub proposal: Option<Proposal> }
  pub fn recognize(p: &Params, tracks: &[TrackInput], library: &LibraryContext) -> Vec<TrackDecision>
  ```
  `recognize` restituisce le decisioni nell'ordine di `tracks`, raggruppando per `evidence.group`.

Album (per ogni file del gruppo e ogni album della sua registrazione migliore): O = 1 se l'album è di tipo `Album` senza tipi secondari e la registrazione ci sta su un'edizione `Official`; titolo = Jaro-Winkler massimo tra il titolo dell'album del file e i titoli dell'album e delle sue edizioni, escluso senza titolo dell'album; unicità = 1 / (numero di album con O = 1 che contengono la registrazione) se O = 1, altrimenti 0. Punteggio del file come per la registrazione; punteggio del gruppo = media sui file (0 per un file la cui registrazione non è in quell'album), come lo spike 4. Indizi di raccolta (`compilation`, oppure artista dell'album uguale a una voce di `various_artists`) e album proposto senza il tipo secondario `Compilation`: l'album è `gated` con `PossibleCompilation` e non passa la soglia (§11 punto 7 del design).

Edizione dentro l'album scelto: copertura = frazione dei file del gruppo la cui registrazione sta nell'edizione; numero di tracce = 1 se `track_count` coincide con il massimo `track_total` del gruppo, escluso se uno dei due manca; in libreria = 1 se l'edizione è in `LibraryContext.releases` per una traccia **fuori** dal gruppo; ufficiale = 1 se `status = Official`. Spareggi: in libreria, data più vecchia (date incomplete dopo quelle complete, senza data in fondo), formato `Digital Media`, mbid.

Decisione (spec §6): nessun candidato: `NotRecognized`; registrazione migliore sotto soglia, con sostanza o con `auto_allowed` falso: `ToUser`; album del gruppo sotto soglia, `gated`, o registrazione del file non in quell'album (`NotOnGroupAlbum`): `ApplyAlbumUnsure`; edizione sotto soglia: `ApplyEditionUnsure`; altrimenti `ApplyAll`. `Proposal` contiene solo i campi che la decisione applicherebbe (tabella §6: con `ApplyAlbumUnsure` niente campi dell'album; con `ApplyEditionUnsure` niente edizione e numero di traccia; anno = anno più vecchio delle edizioni dell'album). I motivi includono sempre quelli di forma e sostanza e quello della soglia mancata.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn woodward_original_album_beats_remixed() { /* stessa impronta, O decide */ }
#[test] fn compilation_hint_blocks_original_album() { /* album artist "Various Artists": ApplyAlbumUnsure, PossibleCompilation */ }
#[test] fn real_compilation_release_group_is_accepted() { /* tipo secondario Compilation e titolo uguale */ }
#[test] fn group_split_file_gets_not_on_group_album() {}
#[test] fn edition_already_in_library_wins_tie() { /* le due edizioni digitali di Brad Sucks */ }
#[test] fn file_without_album_keeps_no_album_when_album_unsure() { /* proposal.album_title None */ }
#[test] fn decision_table() { /* un caso per ogni riga di §6, controlla Decision e campi di Proposal */ }
#[test] fn decisions_keep_input_order() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition --test decision`
Expected: FAIL

- [ ] **Step 3: Implement `recognize`.**

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/armonika-recognition
git commit -m "recognition: album and edition confidence per group and the per-track decision"
```

- [ ] **Step 6: Fine milestone A2**: revisione, push di `phase-2a/a2-engine`, PR in bozza.

---

### Task 8: client AcoustID e MusicBrainz con limiti e cache

**Files:**
- Create: `crates/armonika-core/migrations/0007_lookup_cache.sql`
- Modify: `crates/armonika-core/src/db.rs`, `crates/armonika-core/src/lib.rs` (`user_agent()`), `crates/armonika-scrobble/src/lib.rs` (usa quella del core)
- Create: `crates/armonika-recognition/src/lookup/{mod.rs, limiter.rs, cache.rs, acoustid.rs, musicbrainz.rs}`
- Create: `crates/armonika-recognition/tests/fixtures/` (risposte registrate: una registrazione con più edizioni e accrediti, una senza durata e date, un 404, una ricerca, una richiesta ISRC, una risposta AcoustID)
- Test: `crates/armonika-recognition/tests/lookup.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn armonika_core::user_agent() -> String
  pub struct AcoustIdKey(String);              // Debug e Display scrivono "<redacted>"
  pub struct LookupConfig { pub musicbrainz_url: String, pub acoustid_url: String, pub acoustid_key: AcoustIdKey }
  pub struct AcoustIdMatch { pub recording: String, pub score: f64 }
  pub enum LookupError { Offline { service: &'static str, url: String, message: String },
      Http { service: &'static str, status: u16 }, Parse { service: &'static str, message: String },
      Db(armonika_core::Error) }
  pub struct LookupStats { pub network: u64, pub cached: u64 }
  impl Lookup {
      pub fn new(db: Db, config: LookupConfig) -> Lookup
      pub fn acoustid(&self, fingerprint: &str, duration_s: u64) -> Result<Vec<AcoustIdMatch>, LookupError>
      pub fn recording(&self, mbid: &str) -> Result<Option<Recording>, LookupError>    // 404: None
      pub fn isrc(&self, code: &str) -> Result<Vec<String>, LookupError>              // mbid delle registrazioni
      pub fn search_recordings(&self, title: &str, artist: &str, limit: usize) -> Result<Vec<String>, LookupError>
      pub fn stats(&self) -> LookupStats
  }
  ```
  Tabella `lookup_cache(key TEXT PRIMARY KEY, status INTEGER NOT NULL, body TEXT NOT NULL, fetched_at INTEGER NOT NULL)`.

Richieste: AcoustID `POST {acoustid_url}/v2/lookup` con `client`, `duration`, `fingerprint`, `meta=recordingids`, `format=json` (form urlencoded); per registrazione il punteggio massimo tra i risultati che la contengono. MusicBrainz `GET {musicbrainz_url}/ws/2/recording/<mbid>?inc=releases+release-groups+media+artist-credits+isrcs+aliases&fmt=json`, `GET /ws/2/isrc/<code>?fmt=json`, `GET /ws/2/recording?query=recording:"<t>" AND artist:"<a>"&limit=<n>&fmt=json` (virgolette e caratteri speciali di Lucene escapati). Valori predefiniti: `https://musicbrainz.org`, `https://api.acoustid.org`. Limiti: un `RateLimiter` per servizio con intervallo minimo (1100 ms e 334 ms; i test lo azzerano con un costruttore `#[doc(hidden)]`). MusicBrainz 503: fino a 3 nuovi tentativi dopo 2 s, 4 s, 8 s. Errore di connessione o timeout (20 s): `Offline`. Chiave della cache: servizio + metodo + percorso + parametri ordinati, senza `client`. Se gli alias degli artisti non arrivano nella risposta, il parser li lascia vuoti; il Task 15 verifica su una risposta vera e lo scrive.

- [ ] **Step 1: Write the failing tests** (wiremock, come `crates/armonika-scrobble/tests/listenbrainz.rs`)

```rust
#[test] fn parses_recording_with_releases_credits_isrcs() {}
#[test] fn parses_recording_without_length_and_dates() {}
#[test] fn recording_404_is_none_and_cached() { /* seconda chiamata: 0 richieste al mock */ }
#[test] fn second_lookup_comes_from_cache() { assert_eq!(lookup.stats(), LookupStats { network: 1, cached: 1 }); }
#[test] fn cache_key_and_errors_never_contain_the_key() { /* SELECT key FROM lookup_cache; format!("{:?}", err) */ }
#[test] fn sends_project_user_agent() {}
#[test] fn musicbrainz_requests_are_spaced() { /* limiter vero, 3 richieste: almeno 2,2 s */ }
#[test] fn retries_503_then_succeeds() {}
#[test] fn unreachable_server_is_offline() { /* porta chiusa */ }
#[test] fn acoustid_keeps_best_score_per_recording() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition --test lookup`
Expected: FAIL

- [ ] **Step 3: Implement** il modulo `lookup` con ureq 3 (agent come `ListenBrainz::new`: `http_status_as_error(false)`, timeout 20 s, `user_agent()`), e la migrazione.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates Cargo.lock
git commit -m "recognition: AcoustID and MusicBrainz clients with rate limits and a response cache"
```

---

### Task 9: candidati di una traccia, pacchetto e rapporto

**Files:**
- Create: `crates/armonika-recognition/src/lookup/gather.rs`, `src/package.rs`
- Create: `crates/armonika-core/src/recognition_rows.rs`
- Test: `crates/armonika-recognition/tests/gather.rs`, `tests/package.rs`, `crates/armonika-core/tests/recognition_rows.rs`

**Interfaces:**
- Consumes: Task 8 (`Lookup`), Task 7 (`recognize`, `TrackDecision`), Task 3 (`fingerprints::state`).
- Produces:
  ```rust
  // armonika_core::recognition_rows
  pub struct RecognitionRow { pub track: TrackId, pub rel_path: String, pub title: Option<String>,
      pub artist_credit: Option<String>, pub artists: Vec<String>, pub mb_artist_ids: Vec<String>,
      pub album_title: Option<String>, pub album_artist_credit: Option<String>, pub track_total: Option<u32>,
      pub compilation: bool, pub isrcs: Vec<String>, pub id3v2_major: Option<u8>,
      pub duration_ms: Option<u64>, pub mb_release_id: Option<String>, pub content_hash: Option<Vec<u8>> }
  pub fn library_rows(conn: &Connection, library: LibraryId) -> Result<Vec<RecognitionRow>>   // tracce presenti, ordinate per rel_path
  // armonika_recognition
  pub fn evidence_from_row(row: &RecognitionRow) -> TrackEvidence          // calcola group con group_key
  pub fn library_context(rows: &[RecognitionRow]) -> LibraryContext
  pub fn gather(lookup: &Lookup, p: &Params, ev: &TrackEvidence, fingerprint: Option<&str>) -> Result<Vec<Candidate>, LookupError>
  pub struct Package { pub format_version: u32, pub armonika_version: String, pub library: String,
      pub params_revision: u32, pub created_at_ms: i64, pub context: LibraryContext,
      pub tracks: Vec<TrackInput>, pub decisions: Vec<TrackDecision> }
  pub fn write_package(path: &Path, package: &Package) -> std::io::Result<()>   // file temporaneo poi rename
  pub fn read_package(path: &Path) -> Result<Package, PackageError>              // rifiuta format_version diverso
  pub fn render_report(package: &Package) -> String                             // Markdown
  ```

`gather`: (1) con l'impronta, `acoustid` e per ogni registrazione con punteggio almeno `acoustid_min_candidate` una `recording`; (2) senza candidati e con ISRC, `isrc` per ogni codice e `recording` per ogni risultato; (3) ancora senza candidati, con titolo e artista, `search_recordings(titolo pulito, primo artista del file, text_search_candidates)`. Una `recording` che dà `None` (404) si salta. Doppioni tolti per mbid, tenendo la fonte più forte (AcoustId, poi Isrc, poi TextSearch). Il rapporto: titolo, riepilogo con il numero di tracce per decisione e la percentuale, poi una riga per traccia: `rel_path`, decisione, punteggio della registrazione, album ed edizione, motivi (`Reason::text`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn acoustid_candidates_first() {}
#[test] fn isrc_lookup_only_without_acoustid_results() { /* mock: con risultati AcoustID nessuna chiamata /isrc */ }
#[test] fn text_search_as_last_resort_with_cleaned_title() {}
#[test] fn deleted_recording_is_skipped() {}
#[test] fn package_round_trip_and_version_check() {}
#[test] fn package_has_no_acoustid_key() { /* la chiave del test non compare nel file */ }
#[test] fn report_lists_every_track_with_reasons() {}
#[test] fn library_rows_skip_missing_tracks() {}
#[test] fn library_context_maps_release_ids_to_tracks() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition --test gather --test package && cargo test -p armonika-core --test recognition_rows`
Expected: FAIL

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "recognition: gather candidates per track, write the calibration package and report"
```

---

### Task 10: comando `armonika recognize --dry-run`

**Files:**
- Create: `crates/armonika-server/src/cli/recognize.rs`
- Modify: `crates/armonika-server/src/cli/mod.rs`, `src/main.rs`, `src/config.rs` (`[recognition]` completa), `crates/armonika-server/Cargo.toml`
- Modify: `docs/configuration.md`, `README.md` (sezione breve "Recognition dry run")
- Test: `crates/armonika-server/tests/cli_recognize.rs`

**Interfaces:**
- Consumes: Task 3 (`fingerprints`, `armonika_media::fingerprint`), Task 8, Task 9.
- Produces: comando `armonika recognize --dry-run --library <nome> [--out <cartella>] [--params <file>] [--limit <n>]`; configurazione
  ```toml
  [recognition]
  background_fingerprint = true
  musicbrainz_url = "https://musicbrainz.org"
  acoustid_url = "https://api.acoustid.org"
  acoustid_key_file = "/run/secrets/acoustid"     # facoltativo
  ```
  `ARMONIKA_ACOUSTID_KEY` va in `RESERVED_ENV` (non è più "ignorata": la legge questo comando) e vince sul file.

Comportamento: senza `--dry-run` esce con codice 2 e "only --dry-run is available in this version". Senza chiave: errore che dice come darla. Ordine: legge le tracce (`library_rows`), stampa su stderr il numero di tracce, quante hanno già l'impronta e la stima (`numero di richieste MusicBrainz previste x 1,1 s`, con almeno una richiesta per traccia); calcola e salva le impronte mancanti (`fingerprint` con `config.ffmpeg_path`; i fallimenti vanno in `audio_fingerprint_failures` e la traccia ha `FingerprintStatus::Missing(motivo)`); raccoglie i candidati con avanzamento ogni 10 tracce (`[n/tot] network=x cached=y`); `recognize`; scrive `package.json` e `report.md` nella cartella di uscita (predefinita `<data_dir>/recognition/<nome libreria>`) e stampa il riepilogo per decisione. Un `LookupError::Offline` ferma tutto con "cannot reach <servizio> at <url>: <messaggio>. Recognition needs internet; the server keeps working without it." e codice 1, senza scrivere il pacchetto. `--limit n`: solo le prime n tracce (per `rel_path`).

- [ ] **Step 1: Write the failing tests** (`assert_cmd`, libreria da `armonika-testdata` con 3 file, server finti con wiremock, `ARMONIKA_RECOGNITION__*` puntati ai mock)

```rust
#[test] fn dry_run_writes_package_and_report_and_leaves_files_untouched() { /* hash e mtime dei file uguali prima e dopo */ }
#[test] fn without_dry_run_flag_refuses() {}
#[test] fn missing_key_is_a_clear_error() {}
#[test] fn offline_is_a_clear_error_and_no_package() {}
#[test] fn rerun_after_failure_uses_cache() { /* il mock risponde 500 dopo 2 richieste; secondo giro con mock sano: richieste ricevute = solo quelle mancanti */ }
#[test] fn track_without_fingerprint_still_gets_a_decision() { /* file non decodificabile con TSRC: candidato da /isrc */ }
#[test] fn dry_run_while_another_writer_holds_the_db() { /* un thread tiene una transazione IMMEDIATE per 1 s */ }
#[test] fn prints_estimate_before_starting() {}
#[test] fn key_never_printed() { /* stdout, stderr e file di uscita non contengono la chiave */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-server --test cli_recognize`
Expected: FAIL

- [ ] **Step 3: Implement** `cli/recognize.rs` e la configurazione.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates docs/configuration.md README.md
git commit -m "server: armonika recognize --dry-run writes a report and a calibration package"
```

- [ ] **Step 6: Fine milestone A3**: revisione, push di `phase-2a/a3-dry-run`, PR in bozza.

---

### Task 11: programmino di giudizio

**Files:**
- Create: `bench/recognition-tools/Cargo.toml` (pacchetto `armonika-recognition-tools`, `publish = false`), `src/lib.rs`, `src/judge/{mod.rs, app.rs, ui.rs}`, `src/bin/armonika-judge.rs`
- Create: `crates/armonika-recognition/src/judgment.rs`
- Modify: `Cargo.toml` (members `bench/recognition-tools`), `Dockerfile` (`COPY bench/recognition-tools ./bench/recognition-tools`: senza, `cargo build --locked` nell'immagine fallisce, come nella fase 1 con `bench/harness`), `.github/workflows/docker.yml` (paths)
- Test: `crates/armonika-recognition/tests/judgment.rs`, `bench/recognition-tools/tests/judge_app.rs`

**Interfaces:**
- Consumes: Task 9 (`read_package`, `Package`, `TrackDecision`).
- Produces:
  ```rust
  // armonika_recognition::judgment
  pub enum Verdict { Right, WrongRecording, WrongAlbum, WrongBoth, Unknown }
  pub struct Judgment { pub track: String, pub recording: String, pub release_group: Option<String>,
      pub release: Option<String>, pub decision: Decision, pub verdict: Verdict }
  pub struct Judgments { pub format_version: u32, pub package_created_at_ms: i64, pub sample: Vec<String>, pub items: Vec<Judgment> }
  pub fn sample(package: &Package, p: &Params, size: usize, seed: u64) -> Vec<String>   // id delle tracce
  pub fn read_judgments(path: &Path) -> Result<Judgments, PackageError>
  pub fn write_judgments(path: &Path, j: &Judgments) -> std::io::Result<()>              // temporaneo poi rename
  // judge::app
  pub enum Key { Char(char), Left }
  pub enum Effect { None, Save, OpenUrl(String), Quit }
  pub struct App { .. }  impl App { pub fn new(package: Package, judgments: Judgments) -> App; pub fn handle(&mut self, key: Key) -> Effect; pub fn current(&self) -> Option<&TrackDecision>; pub fn judgments(&self) -> &Judgments }
  ```
  Uso: `armonika-judge <package.json> [--size 130] [--seed 1]`; i giudizi vanno in `judgments.json` nella stessa cartella.

Campione (spec §8): si escludono `NotRecognized`; ogni decisione presente ha almeno `min(10, disponibili)` proposte, il resto in proporzione; dentro ogni decisione estrazione pesata senza ripetizione con `rand_chacha` e `seed`: peso 1, +2 se un punteggio (registrazione, album o edizione) dista al massimo 5 punti dalla sua soglia, +1 se il file o la proposta hanno più di un artista. Meno proposte di `size`: tutte. Il campione si salva in `Judgments.sample` alla prima apertura e non cambia più.

Tasti: `g` Right; `s` poi `1` WrongRecording, `2` WrongAlbum, `3` WrongBoth (altro tasto annulla); `n` Unknown; freccia sinistra o `i` torna alla proposta precedente; `m` `OpenUrl("https://musicbrainz.org/recording/<mbid>")`; `q` Quit. Ogni giudizio dà `Save`; alla riapertura si parte dal primo del campione senza giudizio. Schermo (ratatui + crossterm): in alto "n/totale"; a sinistra tag attuali, a destra proposta, campi diversi evidenziati; sotto punteggi, decisione e motivi; in fondo i tasti.

- [ ] **Step 1: Write the failing tests**

```rust
// judgment.rs
#[test] fn sample_covers_every_decision_and_is_deterministic() {}
#[test] fn sample_prefers_near_threshold_and_multi_artist() { /* su 1000 estrazioni con seed diversi, frequenza maggiore */ }
#[test] fn small_package_gives_everything() {}
// judge_app.rs
#[test] fn g_records_right_and_saves() {}
#[test] fn s_then_2_records_wrong_album() {}
#[test] fn s_then_other_key_cancels() {}
#[test] fn back_changes_a_previous_judgment() {}
#[test] fn m_opens_the_recording_page() {}
#[test] fn reopening_resumes_at_first_unjudged() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition --test judgment && cargo test -p armonika-recognition-tools`
Expected: FAIL

- [ ] **Step 3: Implement** `judgment.rs`, `App` (logica senza terminale) e `ui.rs` (solo disegno e lettura dei tasti; `webbrowser::open` per `OpenUrl`).

- [ ] **Step 4: Run the tests and a manual check**

Run: `cargo test --workspace`, poi `cargo run -p armonika-recognition-tools --bin armonika-judge -- <pacchetto del test del Task 10>` su Windows.
Expected: PASS; lo schermo si apre, i tasti funzionano, `q` esce e `judgments.json` esiste.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock Dockerfile .github/workflows/docker.yml bench/recognition-tools crates/armonika-recognition
git commit -m "bench: armonika-judge, a terminal tool to judge recognition proposals"
```

---

### Task 12: taratura

**Files:**
- Create: `bench/recognition-tools/src/calibrate/{mod.rs, grid.rs, report.rs}`, `src/bin/armonika-calibrate.rs`
- Modify: `crates/armonika-recognition/src/judgment.rs` (verità della libreria disordinata)
- Test: `bench/recognition-tools/tests/calibrate.rs`

**Interfaces:**
- Consumes: Task 7 (`recognize`), Task 9 (`read_package`), Task 11 (`Judgments`).
- Produces:
  ```rust
  // armonika_recognition::judgment
  pub struct TruthEntry { pub file: String, pub recording_mbid: String, pub release_mbid: String, pub also_accept_recordings: Vec<String> }
  pub fn judgments_from_truth(package: &Package, truth: &[TruthEntry]) -> Judgments   // per rel_path; verdetto sulla proposta del pacchetto
  // calibrate
  pub struct Source { pub name: String, pub package: Package, pub judgments: Judgments }
  pub struct Outcome { pub params: Params, pub per_source: Vec<SourceNumbers>, pub holdout: HoldoutNumbers }
  pub fn calibrate(base: &Params, sources: &[Source]) -> Result<Outcome, CalibrateError>   // NoFeasibleParams se nessuna combinazione regge
  ```
  Uso: `armonika-calibrate --source messy=<package>,truth=<ground_truth.json> --source user=<package>,judgments=<judgments.json> [--params <base>] --out <cartella> [--write]`. Scrive `<out>/calibration-report.md` e `<out>/params.toml`; con `--write` sostituisce anche `crates/armonika-recognition/data/params.toml` con `revision` + 1.

Valutazione di una combinazione: si rilancia `recognize` su ogni pacchetto con i parametri della combinazione. Una traccia giudicata "applica la registrazione sbagliata" se la decisione è una delle tre `Apply*` e: il giudizio è `WrongRecording` o `WrongBoth` sulla stessa registrazione, oppure la registrazione scelta ora è diversa da quella giudicata `Right` (una sola registrazione giusta per traccia, salvo `also_accept_recordings`). Se la registrazione scelta ora è diversa da una giudicata sbagliata, la traccia conta come "applicata senza giudizio" (riportata, mai nascosta). Analogo per album (`WrongAlbum`, `WrongBoth` con album applicato) ed edizione.

Griglia della registrazione: pesi acoustid {1, 2, 3}, duration {0, 1, 2, 3}, title {1, 2}, artists {1, 2}, isrc {0, 1, 2, 3}; `title_form_min` {0,90, 0,92, 0,95, 0,98}; soglia da 50 a 100 a passi di 0,5. Vincoli, su **ogni fonte da sola**: zero registrazioni sbagliate applicate; soglia almeno 3 punti sopra il punteggio più alto di una registrazione sbagliata con `auto_allowed` vero e senza sostanza. Controprova: 5 parti per fonte, pesi e soglia scelti sulle altre 4 e provati sulla quinta: zero registrazioni sbagliate applicate in ogni parte di prova. Ordine di scelta tra le combinazioni che rispettano i vincoli: (1) meno tracce `ToUser` in totale; (2) meno tracce "applicate senza giudizio"; (3) margine più grande; (4) somma dei pesi più piccola. Poi, con la registrazione fissata, la stessa ricerca per album (pesi {0, 1, 2} ciascuno, soglia) e per edizione (pesi {0, 1, 2} ciascuno, soglia), con i loro vincoli. Nessuna combinazione possibile: `NoFeasibleParams` con i numeri della migliore, codice di uscita 1.

Rapporto: per fonte e per "tracce con più artisti" (file o proposta con più di un artista): giudizi usati, errori sopra soglia, `ApplyAll`, `ApplyEditionUnsure`, `ApplyAlbumUnsure`, `ToUser`, `NotRecognized`, risultato della controprova, margine; più la frase fissa sui limiti del campione (spec §9: con circa 130 giudizi un tasso d'errore vero fino a circa il 2-3% può non comparire).

- [ ] **Step 1: Write the failing tests** (pacchetti sintetici costruiti nel test)

```rust
#[test] fn truth_marks_wrong_recording_and_near_miss() {}
#[test] fn chosen_threshold_has_3_point_margin_over_worst_wrong() {}
#[test] fn constraint_must_hold_on_each_source_alone() { /* una fonte pulita e una con un errore alto: la soglia sale */ }
#[test] fn holdout_failure_rejects_a_combination() {}
#[test] fn changed_recording_against_right_judgment_counts_as_wrong() {}
#[test] fn no_feasible_params_is_an_error_with_best_numbers() {}
#[test] fn safety_section_is_never_changed() {}
#[test] fn report_has_per_source_and_multi_artist_numbers() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-recognition-tools --test calibrate`
Expected: FAIL

- [ ] **Step 3: Implement.** La griglia gira in parallelo con `rayon` (già nel workspace) se serve; ogni valutazione chiama solo `armonika_recognition::engine::recognize`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add bench/recognition-tools crates/armonika-recognition Cargo.lock
git commit -m "bench: armonika-calibrate searches weights and thresholds under the zero-error constraint"
```

- [ ] **Step 6: Fine milestone A4**: revisione, push di `phase-2a/a4-tools`, PR in bozza.

---

### Task 13: generatore della libreria disordinata, correzioni e danni nuovi

**Files:**
- Modify: `bench/testdata/src/messy.rs`, `bench/testdata/src/bin/gen-messy.rs`, `bench/messy/manifest.toml` (commento del formato), `bench/messy/README.md`
- Test: `bench/testdata/src/messy.rs` (modulo di test), `bench/testdata/tests/messy_build.rs`

**Interfaces:**
- Produces: `GroundTruth.file` diventa un percorso relativo con `/` (es. `e03/t007.flac`) e `GroundTruth` riceve `also_accept_recordings: Vec<String>`; campi nuovi del manifest: `Source.also_accept_recordings` (facoltativo), danni `feat_in_title`, `id3v23_slash`, `remaster_suffix`, `strip_isrc`. Il formato JSON è quello letto da `TruthEntry` del Task 12.

Correzioni note (README, "Known limitations"): i danni che cambiano i tag usano le API native di lofty per tipo di file (`MpegFile`, `FlacFile`, `VorbisFile`, `OpusFile`, `Mp4File`, ...; `split_tag`, modifica, `merge_tag`, salvataggio del file intero), così `DISCC`, `MCN` e `TOTALTRACKS` restano; uscita in cartelle per edizione (`e01`, `e02`, ... in ordine di `release_mbid`; le copie da raccolta in una cartella propria per raccolta) con nomi neutri `t001.<ext>` progressivi, la corrispondenza solo in `ground_truth.json`. Danni nuovi: `feat_in_title` toglie gli artisti dopo il primo dal tag artista e li mette nel titolo come `"<titolo> (feat. <B> & <C>)"`; `id3v23_slash` richiede una sorgente MP3 con più artisti e la salva in ID3v2.3 con `TPE1` uguale agli artisti uniti da `/`; `remaster_suffix` aggiunge `" - Remastered 2011"` al titolo; `strip_isrc` toglie l'ISRC. Ogni danno nuovo è nel commento del formato del manifest.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn mutating_damage_keeps_unknown_fields() { /* flac con DISCC, MCN, TOTALTRACKS: restano dopo wrong_case */ }
#[test] fn output_names_are_neutral_and_grouped_by_edition() { /* nessun nome contiene l'id della sorgente o il danno */ }
#[test] fn feat_in_title_moves_artists() {}
#[test] fn id3v23_slash_writes_v23_with_slash() { /* rilettura: id3v2_major 3, TPE1 "A/B" */ }
#[test] fn remaster_suffix_and_strip_isrc() {}
#[test] fn manifest_accepts_also_accept_recordings() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p armonika-testdata`
Expected: FAIL

- [ ] **Step 3: Implement.** Aggiungere ai sorgenti esistenti di Josh Woodward `also_accept_recordings` con le registrazioni del 2008 (README, "near miss").

- [ ] **Step 4: Run the tests and rebuild the library**

Run: `cargo test --workspace`, poi cancellare `bench/data/messy` e rigenerare con il comando del README.
Expected: PASS; 54 file come prima, in cartelle per edizione, con `ground_truth.json` aggiornato.

- [ ] **Step 5: Commit**

```bash
git add bench/testdata bench/messy
git commit -m "testdata: messy library with native tag writes, neutral names and new damages"
```

---

### Task 14: sorgenti Creative Commons nuove

**Files:**
- Modify: `bench/messy/manifest.toml`, `bench/messy/README.md`

Obiettivo (spec §10): almeno 6 registrazioni con più artisti nell'accredito MusicBrainz (feat. e collaborazioni) di almeno 2 artisti diversi; almeno una raccolta vera (edizione MusicBrainz con artista "Various Artists" e tipo secondario `Compilation`) con almeno 3 tracce nella libreria; almeno 3 registrazioni con ISRC su MusicBrainz; ogni danno nuovo del Task 13 applicato ad almeno 3 file; `id3v23_slash` su sorgenti MP3 con più artisti. Fonti probabili: album di netlabel e raccolte su archive.org o Jamendo che MusicBrainz collega. Per ogni sorgente la stessa verifica delle attuali: licenza dichiarata dall'autore, `sha256`, registrazione sull'edizione (`/ws/2/recording/<id>?inc=releases`), AcoustID con `fpcalc` che dà la registrazione nel primo risultato; riga nella tabella "Provenance and license" con la data.

- [ ] **Step 1: Cerca e verifica le sorgenti**, annotando per ognuna le prove sopra.
- [ ] **Step 2: Aggiorna manifest e README** (tabelle Editions e Provenance, conteggi della libreria, danni per tipo).
- [ ] **Step 3: Rigenera e controlla**

Run: cancellare `bench/data/messy`, rigenerare, `cargo test --workspace`.
Expected: generazione senza errori; il riepilogo del README coincide con `ground_truth.json`.

- [ ] **Step 4: Casi non coperti**: se per un caso non si trovano brani CC adatti, scriverlo nel README ("Known limitations") con le ricerche fatte; il caso si coprirà solo con la libreria vera (Task 16) e il documento della taratura lo dirà.
- [ ] **Step 5: Commit**

```bash
git add bench/messy
git commit -m "testdata: messy library gains multi-artist credits, a real compilation and ISRCs"
```

- [ ] **Step 6: Fine milestone A5**: revisione, push di `phase-2a/a5-messy`, PR in bozza.

---

### Task 15: prova a vuoto e taratura sulla libreria disordinata

Ramo `phase-2a/a6-calibration`. Usa la chiave AcoustID di sviluppo già presente nell'ambiente dell'utente (`ARMONIKA_ACOUSTID_KEY`, come lo spike 4), non quella del progetto.

**Files:**
- Create: `docs/recognition-calibration.md`
- Modify: `crates/armonika-recognition/data/params.toml`
- Modify (solo se servono correzioni trovate qui): codice dei Task 4-12, ognuna con il suo test

- [ ] **Step 1: Prova a vuoto**

Run: libreria `messy` su `bench/data/messy` in un `data_dir` di prova (`armonika library add`), scansione, poi `armonika recognize --dry-run --library messy --out bench/data/recognition-2a/messy`.
Expected: rapporto e pacchetto scritti; un secondo giro stampa `network=0`.

- [ ] **Step 2: Verifica su una risposta vera** che gli alias degli artisti arrivano con `inc=aliases` (Task 8); se no, scriverlo nel documento e lasciare il confronto su nome e nome per l'ordinamento.

- [ ] **Step 3: Taratura sulla sola verità**

Run: `armonika-calibrate --source messy=bench/data/recognition-2a/messy/package.json,truth=bench/data/messy/ground_truth.json --out bench/data/recognition-2a/calib-messy --write`
Expected: codice 0, `params.toml` con `revision = 2`. Codice 1 (`NoFeasibleParams`): fermarsi e portare i numeri all'utente.

- [ ] **Step 4: Scrivi `docs/recognition-calibration.md`** (inglese): come si rifà, numeri della libreria disordinata (per decisione, per tipo di danno, tracce con più artisti, controprova, margine), regole di sicurezza in vigore, limiti; una sezione "Real library" vuota che il Task 16 riempie.

- [ ] **Step 5: Commit**

```bash
git add docs/recognition-calibration.md crates/armonika-recognition/data/params.toml
git commit -m "recognition: weights and thresholds calibrated on the messy library"
```

---

### Task 16: libreria vera, giudizio dell'utente, taratura finale

Prerequisiti, tutti necessari: B6 finito e `docs/phase-1-acceptance.md` su `main` (R35); PR delle milestone A1-A5 unite; l'utente ha registrato l'applicazione Armonika su acoustid.org e ha la chiave. Ogni passo con "l'utente" lo fa l'utente; Claude prepara i comandi.

**Files:**
- Modify: `docs/recognition-calibration.md`, `crates/armonika-recognition/data/params.toml`
- Modify: `docs/superpowers/specs/2026-11-12-armonika-design.md` (§8.2 e §8.6 punto 4)

- [ ] **Step 1: L'utente aggiorna il server di casa** alla nuova immagine e mette la chiave nella configurazione (`ARMONIKA_ACOUSTID_KEY` nel compose o `acoustid_key_file`); attende che i log non mostrino più impronte in corso (oppure lascia che le calcoli il comando).
- [ ] **Step 2: L'utente lancia la prova a vuoto**: `docker exec armonika armonika recognize --dry-run --library <nome>`; copia `package.json` sul PC in una cartella fuori dalla repository (es. `bench/data/recognition-2a/real/`, ignorata da git).
- [ ] **Step 3: L'utente giudica** con `armonika-judge` almeno 100 proposte (accordo: 100-150); può interrompere e riprendere.
- [ ] **Step 4: Taratura finale**

Run: `armonika-calibrate --source messy=...,truth=... --source user=<package>,judgments=<judgments.json> --out bench/data/recognition-2a/calib-final --write`
Expected: codice 0. Codice 1: si torna dall'utente prima del blocco successivo (spec §11).

- [ ] **Step 5: Aggiorna `docs/recognition-calibration.md`** con i numeri aggregati della libreria vera: quante tracce Armonika sistemerebbe da sola (`ApplyAll`), quante con album o edizione da verificare, quante all'utente, non riconosciute; numeri per fonte e per tracce con più artisti; controprova; la frase sui limiti del campione. Nessun titolo, artista o percorso della libreria privata. Controlla con `git diff` che non ci siano.
- [ ] **Step 6: Aggiorna il design principale**: §8.2 (formula e soglie) e §8.6 punto 4 ("applicazione automatica non nella prima versione") sostituiti dalla regola dei due punteggi, con i numeri misurati e il rimando a `docs/recognition-calibration.md`; riga datata.
- [ ] **Step 7: Commit, push, PR**

```bash
git add docs crates/armonika-recognition/data/params.toml
git commit -m "recognition: calibration on the real library and design update"
```
Revisione dell'intera milestone A6, push di `phase-2a/a6-calibration`, PR verso `main`. Con questa PR unita il blocco A è finito (spec §11, punti 1-5).
