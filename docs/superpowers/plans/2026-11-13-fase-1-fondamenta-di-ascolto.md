# Fase 1: fondamenta di ascolto. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** un server Armonika che scansiona una o più librerie, tiene tracce con identità stabile separate dai file, cerca in meno di 50 ms, fa streaming con range HTTP e serve le app Subsonic (navigazione, ricerca, copertine, preferiti, playlist, scrobbling su ListenBrainz) con utenti, librerie e permessi, installabile con Docker.

**Architecture:** workspace Cargo con le crate del design §7.5 che servono in questa fase: `armonika-core` (Glossario, database SQLite, utenti, permessi, dati personali), `armonika-media` (operazioni su un singolo file: tag, copertine, impronta del contenuto, transcodifica), `armonika-scanner` (scansione, collegamento file-traccia, album della libreria, osservazione delle cartelle), `armonika-search` (`SearchIndex` su Tantivy), `armonika-subsonic` (API OpenSubsonic su axum), `armonika-scrobble` (ListenBrainz; Last.fm più avanti), `armonika-server` (eseguibile, configurazione, comandi, wiring). Le API e lo scanner dipendono dal core, mai il contrario; lo scanner non conosce la ricerca: pubblica eventi di cambiamento che il server inoltra all'indice.

**Tech Stack:** Rust 1.99 (edition 2024), tokio, axum, tower-http, rusqlite (`bundled`), r2d2 + r2d2_sqlite, figment, clap, tracing, lofty 0.25 (solo lettura in questa fase), symphonia 0.6.1 (solo demux, per l'impronta del contenuto), blake3, tantivy, notify + notify-debouncer-full, walkdir, image, argon2, chacha20poly1305, uuid (v7), unicode-normalization, reqwest (rustls), ffmpeg esterno opzionale. Test: tempfile, assert_cmd, wiremock, tower (`ServiceExt::oneshot`).

**Spec:** `docs/superpowers/specs/2026-11-12-armonika-design.md`, in particolare §3, §7, §8.1, §8.5, §8.7 (punti 1, 7, 8, 9, 10), §9 (fase 1) e §11. Esiti degli spike in `docs/spikes/`. Chi esegue legge design e piano insieme; il Glossario vale anche qui.

## Decisioni di questo piano (confermate dall'utente il 2026-11-13)

Sono scelte lasciate al builder (§12) o casi che il design non copre. Ognuna è applicata nel task indicato; se l'utente ne cambia una, si cambia solo quel task.

1. **Password Subsonic in fase 1** (Task 3). Finché non c'è l'interfaccia (fase 2), la password Subsonic di un utente è inizialmente uguale alla password dell'account: si salvano sia l'hash argon2 (account) sia una copia cifrata in modo reversibile (Subsonic, §7.4), come fa Navidrome. Si cambia separatamente con `armonika user set-subsonic-password`. Le API key OpenSubsonic sono già disponibili.
2. **Raccolte senza tag "artista dell'album"** (Task 8). Il design raggruppa per titolo dell'album e accredito dell'album; se il tag artista dell'album manca e nella stessa cartella le tracce con lo stesso titolo dell'album hanno artisti diversi, l'accredito dell'album diventa "Various Artists". Senza questa regola una raccolta senza quel tag si spezza in un album per artista.
3. **Chromaprint in background rinviato alla fase 2** (Task 9). In fase 1 il passaggio in background calcola solo l'impronta del contenuto (serve al ricollegamento dei file spostati); l'impronta Chromaprint serve solo ai doppioni e al riconoscimento, che sono della fase 2.
4. **Last.fm rinviato** (Task 19), deciso dall'utente: in fase 1 solo ListenBrainz. Il modulo dello scrobbling resta pronto per un secondo servizio (la colonna `service` ammette già `lastfm`), così aggiungerlo dopo non richiede migrazioni.
5. **Porta predefinita 4747** (Task 1), per non scontrarsi con Navidrome (4533) se l'utente li prova insieme.
6. **Immagine Docker su GHCR privato** finché la repository è privata (Task 21): sul server di casa serve `docker login ghcr.io` con un token personale, oppure si costruisce l'immagine sul posto.
7. **Libreria vera piccola** (Task 25). L'utente ha circa 500-1000 canzoni, non 50.000. Le soglie del §3 restano (Armonika sarà open source e altri hanno librerie grandi), ma si verificano sulla libreria finta generata, anche sull'hardware di casa; la libreria vera serve per l'uso quotidiano e per giudicare a mano la qualità della ricerca, non per le soglie. Disco e macchina di casa per le misure: da decidere con l'utente prima del Task 25.

## Revisione dello schema contro MusicBrainz (2026-11-13, approvata dall'utente)

Lo schema del Task 2 è stato confrontato con le `CREATE TABLE` di MusicBrainz (`admin/sql/CreateTables.sql` di `metabrainz/musicbrainz-server`). Cambi adottati, ognuno nel task indicato:

1. **Reindirizzamenti dopo le unioni** (come `*_gid_redirect`): tabella `id_redirects`, scritta da ogni unione di tracce (Task 9) e di album della libreria (Task 8) e consultata da ogni ricerca per identificativo (Task 2, Task 12). Le app Subsonic salvano gli identificativi nei dispositivi; dopo un'unione devono continuare a trovarli.
2. **Date parziali, edizione e prima pubblicazione** (come `release_country.date_*` e `recording_first_release_date`): `date_*` e `orig_date_*` (Task 5), date dell'album (Task 8), `releaseDate` e `originalReleaseDate` nelle API (Task 13).
3. **Numero di traccia come testo** (come `track.number TEXT` accanto a `position`): `track_number_text` più `track_no` per ordinare; `disc_subtitle` (come `medium.name`) (Task 5, Task 13).
4. **Nome accreditato e testo di unione** (come `artist_credit_name.name` e `join_phrase`): in `track_artists` e `album_artists` (Task 6).
5. **`TAG_SCHEMA_VERSION`**: un tag letto in più in futuro fa rileggere i file una volta sola, invece di restare vuoto sui file che la scansione incrementale salta (Task 5, Task 7).
6. **Nomi per l'ordinamento** (come `sort_name`): dai tag `*SORT` (Task 5, Task 6, Task 13).
7. **`updated_at` su album e artisti** (come `last_updated`), per `ifModifiedSince` (Task 6, Task 8, Task 13).
8. **Chiave interna intera e identificativo pubblico** (come `id SERIAL` e `gid UUID`): `id INTEGER PRIMARY KEY` per i collegamenti, `public_id` per le API (Task 2).

Non adottati (da aggiungere con il punto 5 quando servono): alias multilingue degli artisti, più canzoni per registrazione, ISRC, etichetta, numero di catalogo, codice a barre, paese, tipo e stato dell'edizione.

## Global Constraints

- Licenza AGPL-3.0; ogni crate nuova ha `license.workspace = true`, `edition.workspace = true`, `version.workspace = true`, `repository.workspace = true` e `[lints] workspace = true`.
- Codice, commenti, messaggi di commit, README e documentazione pubblica in inglese; questo piano e i documenti in `docs/superpowers/` possono restare in italiano.
- Niente segni tipici del testo generato: lineette lunghe (U+2014, U+2013), frecce (U+2192), puntini di sospensione come carattere unico (U+2026), virgolette tipografiche (U+201C, U+201D, U+2018, U+2019). Prima di ogni commit: `python scripts/check_text_style.py` esce con codice 0.
- Nessuna menzione di Claude come autore o co-autore in commit, PR o file.
- Prima di ogni commit: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- Rami: ogni milestone (M1-M6 sotto) su un proprio ramo `phase-1/<milestone>` partito da `main` aggiornato; a fine milestone pull request verso `main`, unita dall'utente (squash) solo con CI verde. Mai push diretti su `main`.
- Nomi dei tipi nel codice dal Glossario (§7.5): `Track`, `Library`, `LibraryAlbum`, `Artist`, `Recording`, `Release`, `ReleaseGroup`, `Work`. Una traccia non è mai chiamata `File` e viceversa.
- **I dati musicali di una traccia sono sempre e solo i tag del suo file**: le colonne di tag nel database sono una copia per velocità, che la scansione riallinea; nessun comando o endpoint di questa fase le modifica in altro modo.
- **Nessun file di una libreria viene modificato, spostato o cancellato in questa fase.** Lofty si usa solo in lettura (mai `save_to_path`); la cartella `.armonika/` non si crea. La scansione ignora `.armonika/` in ogni libreria.
- Multipiattaforma: percorsi solo con `std::path`; nel database i percorsi relativi alla radice della libreria si salvano con separatore `/` (funzione unica `rel_path_to_db` / `rel_path_from_db` in `armonika-core`).
- Formati accettati dalla scansione (§7.2): estensioni `mp3`, `flac`, `ogg`, `opus`, `m4a`, `wav`, `aif`, `aiff`, confrontate senza distinguere maiuscole.
- Segreti mai committati né scritti nei log: password, chiave di cifratura, API key, token ListenBrainz.
- Soglie (§3), verificate dal Task 23: ricerca p95 < 50 ms lato server su 100.000 tracce con 1 errore di battitura; primo byte audio < 200 ms senza transcodifica; prima scansione di 50.000 file: obiettivo indicativo < 5 min su HDD (§11 punto 5); scansioni successive rileggono solo i file con dimensione o data di modifica cambiate; RAM a riposo indicativa < 100 MB con 100.000 tracce.
- User-Agent per servizi esterni: `Armonika/<versione> ( https://github.com/filocorsa/Armonika )`.

## Review Focus

1. **Disco non montato durante una scansione.** Se la radice di una libreria non esiste o non si legge, la scansione si interrompe con un errore nei log e **nessuna** traccia diventa mancante. Test in Task 7 (`unavailable_root_aborts_without_marking_missing`).
2. **Sincronizzazione completa di Symfonium**: chiama `search3` con `query=""` (o `""` tra virgolette) e pagine da 500 con `songOffset` per scaricare tutta la libreria. Deve restituire tutte le tracce visibili, in ordine stabile, senza duplicati tra pagine. Test in Task 14 (`empty_query_pages_whole_library`).
3. **Rinomina che cambia solo maiuscole/minuscole** (`Song.mp3` in `song.mp3`) su Windows e macOS: la traccia mantiene il suo identificativo. Test in Task 7 (`case_only_rename_keeps_track`).
4. **File in copia durante la scansione** (troncato, tag illeggibili): non fa fallire la scansione, non crea una traccia, viene ripreso alla scansione successiva. Test in Task 7 (`unreadable_file_is_skipped_and_retried`).
5. **Collegamento simbolico che crea un ciclo** dentro la cartella di una libreria: la scansione termina e segnala il ciclo nei log. Test in Task 7 (`symlink_loop_does_not_hang`, solo dove i symlink si creano senza privilegi: Linux e macOS; su Windows il test si salta).

---

## Struttura dei file

```
crates/armonika-core/
  migrations/0001_initial.sql      schema completo della fase 1
  src/ids.rs                       identificativi stabili
  src/text.rs                      normalizzazione dei nomi
  src/paths.rs                     percorsi relativi nel database, controllo sovrapposizioni
  src/db.rs                        Db: scrittore unico, lettori in pool, migrazioni
  src/secret.rs                    SecretBox: cifratura reversibile (password Subsonic, token)
  src/users.rs                     utenti, password, API key
  src/access.rs                    Access: permessi di un utente sulle librerie
  src/libraries.rs                 librerie, sincronizzazione con la configurazione
  src/model.rs                     FileFacts, TrackRow, eventi di cambiamento
  src/artists.rs                   separazione degli accrediti, identità degli artisti
  src/albums.rs                    chiave di raggruppamento, identità degli album della libreria
  src/tracks.rs                    lettura delle tracce visibili (usata dalle API)
  src/annotations.rs               preferiti, valutazioni, ascolti, cronologia
  src/playlists.rs                 playlist con voci nascoste (§8.7 punto 9)
  src/play_queue.rs                coda di riproduzione salvata, in ascolto ora
crates/armonika-media/
  src/probe.rs                     lettura di tag e proprietà (lofty, sola lettura)
  src/cover.rs                     copertine incorporate e di cartella, ridimensionamento
  src/content_hash.rs              BLAKE3 sui pacchetti compressi (Symphonia)
  src/transcode.rs                 transcodifica con ffmpeg esterno
crates/armonika-scanner/
  src/walk.rs                      visita della cartella della libreria
  src/reconcile.rs                 collegamento file-traccia, mancanti, pre-filtro
  src/hasher.rs                    impronte in background, ricollegamento
  src/controller.rs                ScanController: una scansione per libreria, stato
  src/watch.rs                     osservazione delle cartelle e scansione periodica
crates/armonika-search/
  src/lib.rs                       trait SearchIndex, tipi dei risultati
  src/tantivy_index.rs             implementazione Tantivy
  src/cjk.rs                       tokenizer a coppie di caratteri CJK
  src/rerank.rs                    riordino comune (porting da spikes/01-search/src/common.rs)
  src/updater.rs                   applica gli eventi di cambiamento all'indice
crates/armonika-subsonic/
  src/lib.rs                       router `/rest/*`
  src/auth.rs, src/response.rs, src/error.rs
  src/system.rs, src/browse.rs, src/search.rs, src/media.rs, src/annotate.rs, src/playlists.rs
crates/armonika-scrobble/
  src/lib.rs, src/listenbrainz.rs, src/outbox.rs
crates/armonika-server/
  src/main.rs, src/config.rs, src/cli/{user.rs,library.rs,track.rs}, src/serve.rs
bench/testdata/src/fixture.rs      file audio con tag per i test delle altre crate
bench/harness/                     crate armonika-bench: misura le soglie del §3
Dockerfile, docker-compose.example.yml, .github/workflows/docker.yml
docs/benchmarks.md                 come si misura, risultati
docs/phase-1-acceptance.md         esito dell'uscita della fase
```

---

## M1. Fondamenta (ramo `phase-1/m1-foundations`)

### Task 1: Configurazione, log e comando `serve`

**Files:**
- Create: `crates/armonika-server/src/config.rs`, `crates/armonika-server/src/serve.rs`
- Modify: `crates/armonika-server/src/main.rs`, `crates/armonika-server/Cargo.toml`
- Test: `crates/armonika-server/tests/config.rs`, `crates/armonika-server/tests/cli.rs`

**Interfaces:**
- Produces: `Config::load(path: Option<&Path>) -> anyhow::Result<Config>`; campi:
  - `data_dir: PathBuf` (default `data`), `server.bind: SocketAddr` (default `0.0.0.0:4747`), `web.enabled: bool` (default `true`);
  - `scan.watch: bool` (default `true`), `scan.interval: humantime Duration` (default `1h`, `0s` = mai), `scan.threads: usize` (default `4`), `scan.hash_on_start: bool` (default `true`);
  - `libraries: Vec<LibraryConfig { name: String, path: PathBuf }>` (TOML `[[library]]`);
  - `ffmpeg_path: Option<PathBuf>` (se assente si cerca `ffmpeg` nel `PATH`);
  - `transcode.format: String` (default `mp3`), `transcode.bitrate_kbps: u32` (default `192`);
  - `search.ignored_articles: String` (default `The El La Los Las Le Les Il Lo Gli I`);
  - `listenbrainz.url: String` (default `https://api.listenbrainz.org`).
- Fonti in ordine di precedenza crescente: default, file TOML (`--config <path>` oppure `ARMONIKA_CONFIG`, altrimenti `armonika.toml` se esiste), variabili `ARMONIKA_*` con `__` come separatore dei livelli (`ARMONIKA_SERVER__BIND`, `ARMONIKA_DATA_DIR`). Libreria figment.
- CLI (clap): `armonika serve`, e i gruppi `user`, `library`, `track` aggiunti dai task successivi. Senza sottocomando si stampa l'aiuto. `--version` resta come nel test esistente.

- [ ] **Step 1: Test che falliscono** in `tests/config.rs`: `defaults_without_file` (bind `0.0.0.0:4747`, data_dir `data`, interval 1 h), `file_overrides_defaults`, `env_overrides_file` (`ARMONIKA_SERVER__BIND=127.0.0.1:9000`), `libraries_from_toml` (due `[[library]]`), `unknown_key_is_rejected` (errore che nomina la chiave). In `tests/cli.rs`: `serve_answers_on_bind_address`: avvia `armonika serve` con `ARMONIKA_SERVER__BIND=127.0.0.1:<porta libera>` e `ARMONIKA_DATA_DIR=<tempdir>`, attende fino a 10 s che `GET /` risponda, poi termina il processo.
- [ ] **Step 2:** `cargo test -p armonika-server`: FAIL (moduli mancanti).
- [ ] **Step 3: Implementare** `config.rs` con `#[serde(deny_unknown_fields)]`, logging con `tracing-subscriber` (`RUST_LOG`, default `info`), `serve.rs` con axum su `server.bind`, crea `data_dir` se manca, risponde a `GET /` con 200 e testo `Armonika <versione>` se `web.enabled`, 404 altrimenti. Chiusura pulita su Ctrl+C e SIGTERM.
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(server): configuration file, environment overrides and serve command`

### Task 2: Database, schema e identificativi

**Files:**
- Create: `crates/armonika-core/migrations/0001_initial.sql`, `src/db.rs`, `src/ids.rs`, `src/text.rs`, `src/paths.rs`
- Modify: `crates/armonika-core/src/lib.rs`, `crates/armonika-core/Cargo.toml`
- Test: `crates/armonika-core/tests/db.rs`, `tests/text.rs`

**Interfaces:**
- Produces:
  - `Db::open(path: &Path) -> Result<Db>`: WAL, `foreign_keys=ON`, `busy_timeout=5000`, `synchronous=NORMAL`; applica le migrazioni. `Db` è `Clone` (Arc interno): uno scrittore (`Mutex<Connection>`) e un pool r2d2 di 4 lettori.
  - `Db::read<T: Send + 'static>(&self, f: impl FnOnce(&Connection) -> Result<T> + Send + 'static) -> Result<T>` (async, `spawn_blocking`), `Db::write` uguale ma con `&mut Connection`; `Db::read_blocking`, `Db::write_blocking` per scanner e CLI.
  - Migrazioni: file `migrations/NNNN_*.sql` incorporati con `include_str!`, versione in `PRAGMA user_version`, ognuna in una transazione. Niente crate esterne per le migrazioni.
  - `ids.rs`: newtype degli **identificativi pubblici** `TrackId`, `LibraryAlbumId`, `ArtistId`, `UserId`, `PlaylistId` (String) con `::generate()` = prefisso + UUID v7 in esadecimale senza trattini (`t`, `al`, `ar`, `u`, `pl` seguiti da `-`, es. `t-0193...`), `as_str()`, `FromStr` che valida il prefisso; `LibraryId(i64)` (le app Subsonic si aspettano un intero per le cartelle musicali). Nel database sono la colonna `public_id`; i collegamenti tra tabelle usano la chiave interna `id INTEGER`.
  - **Regola di confine:** le funzioni pubbliche del core ricevono e restituiscono identificativi pubblici; la traduzione nella chiave interna avviene dentro il core con `ids::resolve_track(conn, &TrackId) -> Result<Option<i64>>` (e `resolve_album`, `resolve_artist`, `resolve_user`, `resolve_playlist`). Se il `public_id` non esiste, `resolve_*` lo cerca in `id_redirects` e restituisce l'entità verso cui punta. Le chiavi interne non escono mai dal core e non compaiono nelle API.
  - `ids::add_redirect(tx, old: &str, new: &str)`: scrive il reindirizzamento e aggiorna quelli che puntavano a `old` perché puntino a `new` (catena sempre lunga uno, come i `*_gid_redirect` di MusicBrainz). Lo usano tutte le unioni (Task 8 per gli album, Task 9 per le tracce).
  - `text::normalize(s: &str) -> String`: NFKD, via i segni diacritici, minuscole, ogni carattere non alfanumerico diventa spazio, spazi compressi e tolti ai bordi (stessa normalizzazione dello spike 4). `text::fold_sort(s: &str, articles: &[&str]) -> String`: chiave di ordinamento senza articolo iniziale.
  - `paths::rel_path_to_db(rel: &Path) -> String` (separatore `/`), `paths::rel_path_from_db(s: &str) -> PathBuf`.

Schema (`0001_initial.sql`). Tempi in millisecondi Unix (`INTEGER`). Questo è il contratto per tutti i task successivi:

```sql
-- Convenzione (revisione contro lo schema di MusicBrainz, punto 8): ogni entità ha una
-- chiave interna `id INTEGER PRIMARY KEY` usata da tutti i collegamenti e un
-- `public_id TEXT UNIQUE` stabile, l'unico esposto nelle API.
CREATE TABLE users (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,              -- argon2id, formato PHC
  subsonic_password BLOB NOT NULL,          -- SecretBox
  is_admin INTEGER NOT NULL DEFAULT 0, can_download INTEGER NOT NULL DEFAULT 1,
  share_now_playing INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
CREATE TABLE api_keys (
  key_hash TEXT PRIMARY KEY,                -- blake3 esadecimale della chiave
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  label TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE libraries (
  id INTEGER PRIMARY KEY,                   -- esposto così com'è: le app vogliono un intero
  name TEXT NOT NULL UNIQUE, root TEXT NOT NULL,
  write_level TEXT NOT NULL DEFAULT 'read_only'
    CHECK (write_level IN ('read_only','tags','tags_and_organize')),
  created_at INTEGER NOT NULL);
CREATE TABLE library_access (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
  permission TEXT NOT NULL CHECK (permission IN ('listen','manage')),
  PRIMARY KEY (user_id, library_id));
CREATE TABLE artists (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL, norm_name TEXT NOT NULL, sort_name TEXT, mbid TEXT UNIQUE,
  updated_at INTEGER NOT NULL);
CREATE UNIQUE INDEX artists_name_only ON artists(norm_name) WHERE mbid IS NULL;
CREATE TABLE library_albums (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
  group_key TEXT NOT NULL, title TEXT NOT NULL, sort_title TEXT, artist_credit TEXT NOT NULL,
  release_mbid TEXT, release_group_mbid TEXT, is_singles INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX library_albums_key ON library_albums(library_id, group_key);
CREATE TABLE album_artists (
  album_id INTEGER NOT NULL REFERENCES library_albums(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, artist_id INTEGER NOT NULL REFERENCES artists(id),
  credited_name TEXT NOT NULL, join_phrase TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (album_id, position));
CREATE TABLE tracks (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK (state IN ('present','missing','deleted')),
  rel_path TEXT NOT NULL, size INTEGER NOT NULL, mtime_ms INTEGER NOT NULL,
  tag_schema INTEGER NOT NULL,              -- TAG_SCHEMA_VERSION con cui sono stati letti i tag
  format TEXT NOT NULL, duration_ms INTEGER, bitrate_kbps INTEGER, sample_rate INTEGER,
  channels INTEGER, bit_depth INTEGER,
  content_hash BLOB, hash_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (hash_state IN ('pending','done','failed')),
  prev_content_hash BLOB,                   -- impronta prima di un ricollegamento da pre-filtro
  prev_rel_path TEXT,                       -- percorso prima di un ricollegamento da pre-filtro
  title TEXT, sort_title TEXT, artist_credit TEXT, artist_sort TEXT,
  album_title TEXT, album_sort TEXT, album_artist_credit TEXT, album_artist_sort TEXT,
  track_no INTEGER,                         -- per ordinare; NULL se il tag non è un numero
  track_number_text TEXT,                   -- valore originale del tag (es. "A1" dei vinili)
  track_total INTEGER, disc_no INTEGER, disc_total INTEGER, disc_subtitle TEXT,
  date_y INTEGER, date_m INTEGER, date_d INTEGER,                 -- data dell'edizione
  orig_date_y INTEGER, orig_date_m INTEGER, orig_date_d INTEGER,  -- prima pubblicazione
  compilation INTEGER NOT NULL DEFAULT 0, has_embedded_cover INTEGER NOT NULL DEFAULT 0,
  mb_recording_id TEXT, mb_release_track_id TEXT, mb_release_id TEXT,
  mb_release_group_id TEXT, mb_work_id TEXT,
  library_album_id INTEGER REFERENCES library_albums(id),
  added_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, missing_since INTEGER);
CREATE UNIQUE INDEX tracks_present_path ON tracks(library_id, rel_path) WHERE state = 'present';
CREATE INDEX tracks_album ON tracks(library_album_id);
CREATE INDEX tracks_prefilter ON tracks(library_id, size, mtime_ms) WHERE state = 'missing';
CREATE INDEX tracks_hash ON tracks(library_id, content_hash);
CREATE INDEX tracks_hash_pending ON tracks(hash_state) WHERE hash_state = 'pending';
CREATE TABLE track_artists (
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, artist_id INTEGER NOT NULL REFERENCES artists(id),
  credited_name TEXT NOT NULL, join_phrase TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (track_id, position));
CREATE TABLE track_genres (
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, genre TEXT NOT NULL,
  PRIMARY KEY (track_id, genre));
CREATE TABLE id_redirects (                 -- come i *_gid_redirect di MusicBrainz
  old_public_id TEXT PRIMARY KEY, new_public_id TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE track_annotations (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  starred_at INTEGER, rating INTEGER CHECK (rating BETWEEN 1 AND 5),
  play_count INTEGER NOT NULL DEFAULT 0, last_played_at INTEGER,
  PRIMARY KEY (user_id, track_id));
CREATE TABLE album_annotations (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  album_id INTEGER NOT NULL REFERENCES library_albums(id) ON DELETE CASCADE,
  starred_at INTEGER, rating INTEGER CHECK (rating BETWEEN 1 AND 5),
  PRIMARY KEY (user_id, album_id));
CREATE TABLE artist_annotations (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
  starred_at INTEGER, rating INTEGER CHECK (rating BETWEEN 1 AND 5),
  PRIMARY KEY (user_id, artist_id));
CREATE TABLE play_history (
  id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, played_at INTEGER NOT NULL);
CREATE TABLE playlists (
  id INTEGER PRIMARY KEY, public_id TEXT NOT NULL UNIQUE,
  owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL, comment TEXT, public INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE playlist_entries (
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  position INTEGER NOT NULL, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  PRIMARY KEY (playlist_id, position));
CREATE TABLE play_queues (
  user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  current_index INTEGER, position_ms INTEGER NOT NULL DEFAULT 0,
  changed_at INTEGER NOT NULL, changed_by TEXT NOT NULL);
CREATE TABLE play_queue_entries (
  user_id INTEGER NOT NULL REFERENCES play_queues(user_id) ON DELETE CASCADE,
  position INTEGER NOT NULL, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, position));
CREATE TABLE now_playing (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, client TEXT NOT NULL,
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE, started_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, client));
CREATE TABLE scrobble_accounts (
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  service TEXT NOT NULL CHECK (service IN ('lastfm','listenbrainz')),
  secret BLOB NOT NULL, account_name TEXT, PRIMARY KEY (user_id, service));
CREATE TABLE scrobble_outbox (
  id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  service TEXT NOT NULL, track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  played_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at INTEGER NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

Le tracce `deleted` non nascono in questa fase (il cestino è della fase 2), ma lo stato esiste già perché tutte le query di visibilità lo devono escludere.

- [ ] **Step 1: Test che falliscono** in `tests/db.rs`: `open_applies_migrations_once` (due `open` sullo stesso file, `user_version` = 1), `foreign_keys_are_enforced` (inserire `library_access` con utente inesistente fallisce), `wal_mode_is_on`, `present_path_is_unique_but_missing_is_not` (due tracce `missing` con lo stesso `rel_path` ammesse, due `present` no), `readers_see_committed_writes`. In `tests/text.rs`: `normalize("  Beyoncé  Knowles!") == "beyonce knowles"`, `normalize("Sigur Rós") == "sigur ros"`, `normalize("AC/DC") == "ac dc"`, `normalize("坂本龍一") == "坂本龍一"`; `ids_roundtrip_and_reject_wrong_prefix`; `rel_path_uses_forward_slashes`; `resolve_follows_redirect`; `redirect_chains_are_flattened` (A verso B, poi B verso C: A risolve in C con una sola lettura di `id_redirects`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** i moduli.
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(core): sqlite database with migrations, phase 1 schema and stable ids`

### Task 3: Segreti, utenti, accesso e comandi `user`

**Files:**
- Create: `crates/armonika-core/src/secret.rs`, `src/users.rs`, `src/access.rs`, `crates/armonika-server/src/cli/user.rs`
- Test: `crates/armonika-core/tests/users.rs`, `crates/armonika-server/tests/cli_user.rs`

**Interfaces:**
- Produces:
  - `SecretBox::load_or_create(data_dir: &Path) -> Result<SecretBox>`: chiave di 32 byte da `ARMONIKA_SECRET_KEY` (base64) se presente, altrimenti dal file `data_dir/secret.key` (creato al primo avvio con permessi 0600 dove esistono). `seal(&self, plain: &[u8]) -> Vec<u8>` (XChaCha20-Poly1305, nonce casuale di 24 byte in testa), `open(&self, sealed: &[u8]) -> Result<Vec<u8>>`.
  - `users::create(conn, sb: &SecretBox, NewUser { name, password, is_admin, can_download }) -> Result<UserId>` (password minima 8 caratteri; la password Subsonic iniziale è la stessa, decisione 1); `verify_password(conn, name, password) -> Result<Option<UserId>>`; `subsonic_password(conn, sb, &UserId) -> Result<String>`; `set_password`, `set_subsonic_password`, `set_flags(conn, &UserId, UserFlagsUpdate { is_admin: Option<bool>, can_download: Option<bool>, share_now_playing: Option<bool> })`; `delete(conn, &UserId) -> Result<()>` (errore `LastAdmin` se è l'ultimo admin; lo stesso vale per togliere `is_admin` all'ultimo); `by_name`; `list`.
  - `users::create_api_key(conn, &UserId, label) -> Result<String>` (32 byte casuali in base64url; si salva solo l'hash blake3), `user_by_api_key(conn, key) -> Result<Option<UserId>>`, `revoke_api_key`.
  - `users::bootstrap_admin(conn, sb, name: Option<String>, password: Option<String>) -> Result<BootstrapOutcome>`: se non esiste nessun admin e `ARMONIKA_ADMIN_USER` / `ARMONIKA_ADMIN_PASSWORD` sono presenti, crea l'admin; se non esiste nessun admin e mancano, `serve` scrive nei log come crearlo e continua (le API rispondono errore 40 a tutti).
  - `access::grant(conn, &UserId, LibraryId, Permission)`, `revoke`; `Permission { Listen, Manage }`.
  - `Access { user: UserId, name: String, is_admin: bool, can_download: bool, libraries: Vec<(LibraryId, Permission)> }`, `Access::load(conn, &UserId) -> Result<Access>`: per un admin `libraries` contiene tutte le librerie con `Manage`. `Access::visible(&self, filter: Option<LibraryId>) -> Vec<LibraryId>` (filtro su una libreria non accessibile = lista vuota, mai errore che ne riveli l'esistenza).
  - CLI: `armonika user create <name> [--admin] [--no-download]` (password da `--password-stdin` o richiesta a terminale, mai come argomento), `user list`, `user delete <name>`, `user set-password <name>`, `user set-subsonic-password <name>`, `user show-subsonic-password <name>`, `user set <name> [--admin true|false] [--download true|false] [--share-now-playing true|false]`, `user grant <name> <library> listen|manage`, `user revoke <name> <library>`, `user api-key create <name> <label>` (stampa la chiave una volta), `user api-key revoke <name> <label>`.

- [ ] **Step 1: Test che falliscono** (core): `seal_open_roundtrip`, `open_rejects_tampered_ciphertext`, `secret_key_file_is_reused_across_loads`, `create_and_verify_password`, `wrong_password_is_rejected`, `subsonic_password_is_recoverable`, `cannot_delete_last_admin`, `cannot_demote_last_admin`, `api_key_lookup_uses_hash_only` (nella tabella non compare la chiave in chiaro), `admin_sees_all_libraries_with_manage`, `new_user_sees_no_library` (§8.5), `visible_with_foreign_filter_is_empty`. CLI: `user_create_then_list` (con `--password-stdin`), `password_never_in_output`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.** argon2id con i parametri predefiniti del crate `argon2`.
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(core): users, permissions, api keys and user commands`

### Task 4: Librerie

**Files:**
- Create: `crates/armonika-core/src/libraries.rs`, `crates/armonika-server/src/cli/library.rs`
- Modify: `crates/armonika-core/src/paths.rs`, `crates/armonika-server/src/serve.rs`
- Test: `crates/armonika-core/tests/libraries.rs`, `crates/armonika-server/tests/cli_library.rs`

**Interfaces:**
- Produces:
  - `paths::check_library_root(new: &Path, other_roots: &[PathBuf], data_dir: &Path) -> Result<(), RootError>`: confronto su percorsi canonicalizzati (`dunce::canonicalize` su Windows, per evitare i prefissi `\\?\`), senza distinguere maiuscole su Windows e macOS. Errori: `Overlaps(other)` se una contiene l'altra o coincidono, `ContainsDataDir`, `InsideDataDir`, `NotADirectory`.
  - `libraries::create(conn, name, root: &Path, data_dir) -> Result<LibraryId>`, `rename`, `set_root` (mantiene l'identificativo, §8.7 punto 8), `remove(conn, LibraryId) -> Result<RemovalSummary { tracks, playlists_touched }>` (cancella dal database tracce e dati collegati, mai file), `list`, `by_name`.
  - `libraries::sync_from_config(conn, &[LibraryConfig], data_dir) -> Result<Vec<SyncNote>>`: crea solo le librerie il cui nome non esiste; se il percorso nella configurazione è diverso da quello nel database vince il database e si restituisce una nota che `serve` scrive nei log come `warn`.
  - CLI: `armonika library add <name> <path>`, `library list`, `library rename <old> <new>`, `library set-path <name> <path>`, `library remove <name> --yes` (senza `--yes` stampa il riepilogo e non fa nulla).

- [ ] **Step 1: Test che falliscono:** `nested_roots_are_rejected` (in entrambe le direzioni), `same_root_different_case_rejected_on_case_insensitive_os` (`#[cfg(any(windows, target_os = "macos"))]`), `data_dir_inside_library_rejected`, `config_creates_only_missing_libraries`, `database_wins_over_config_path` (con nota), `set_root_keeps_id`, `remove_keeps_files_on_disk` (crea un file nella radice, rimuove la libreria, il file esiste ancora). CLI: `remove_without_yes_changes_nothing`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare**; `serve` chiama `sync_from_config` e `bootstrap_admin` all'avvio.
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(core): libraries with overlap checks and library commands`
- [ ] **Step 6: Pull request M1** verso `main` con titolo `Phase 1 M1: foundations`.

---

## M2. Scansione (ramo `phase-1/m2-scanner`)

### Task 5: Lettura dei file (`armonika-media::probe`) e file di prova

**Files:**
- Create: `crates/armonika-media/` (Cargo.toml, `src/lib.rs`, `src/probe.rs`), `bench/testdata/src/fixture.rs`
- Modify: `bench/testdata/src/lib.rs`, `crates/armonika-core/src/model.rs`
- Test: `crates/armonika-media/tests/probe.rs`

**Interfaces:**
- Produces:
  - In `armonika-core::model`: `FileFacts { size: u64, mtime_ms: i64, format: AudioFormat, duration_ms: Option<u64>, bitrate_kbps: Option<u32>, sample_rate: Option<u32>, channels: Option<u8>, bit_depth: Option<u8>, tags: TagFacts }`; `TagFacts { title, title_sort, artist_credit, artist_sort, artists: Vec<String> /* tag ARTISTS */, album_title, album_sort, album_artist_credit, album_artist_sort, album_artists: Vec<String>, track_no: Option<u32>, track_number_text, track_total, disc_no, disc_total, disc_subtitle, date: Option<PartialDate>, original_date: Option<PartialDate>, genres: Vec<String>, compilation: bool, has_embedded_cover: bool, mb_recording_id, mb_release_track_id, mb_release_id, mb_release_group_id, mb_work_id, mb_artist_ids: Vec<String>, mb_album_artist_ids: Vec<String> }` (stringhe `Option<String>`, vuote = `None`, spazi tolti ai bordi); `PartialDate { year: i32, month: Option<u8>, day: Option<u8> }` (date parziali come in MusicBrainz); `AudioFormat { Mp3, Flac, Ogg, Opus, M4a, Wav, Aiff }` con `from_extension(&str) -> Option<AudioFormat>`.
  - `armonika_media::probe::TAG_SCHEMA_VERSION: u32 = 1`: si incrementa ogni volta che cambia l'insieme dei tag letti o il modo di interpretarli; il Task 7 rilegge una volta tutte le tracce lette con una versione precedente. Così un tag aggiunto in una fase successiva (etichetta, numero di catalogo, tipo di edizione) non resta vuoto sui file che la scansione incrementale salterebbe.
  - `armonika_media::probe::read_file(path: &Path) -> Result<FileFacts, ProbeError>`: lofty `Probe::open(path)?.guess_file_type()?.read()` (sola lettura), tag primario del formato con ripiego sugli altri tag presenti. `date` da `RecordingDate`/`Year` (`TDRC`, `DATE`, `©day`), `original_date` da `OriginalReleaseDate` (`TDOR`, `ORIGINALDATE`; se manca, `ORIGINALYEAR`): si accettano `YYYY`, `YYYY-MM`, `YYYY-MM-DD` e testi che iniziano con 4 cifre (`"1975-03-01T00:00"` dà 1975-03-01, `"1975?"` dà 1975). Numero di traccia: `track_number_text` è il valore del tag senza il totale (`"A1"`, `"3"`), `track_no` è il numero se il valore è un intero, altrimenti `None`; per i lati dei vinili (`A1`, `B2`) `track_no` resta `None` e l'ordine lo dà il Task 13. `disc_subtitle` da `TSST` / `DISCSUBTITLE` / `----:com.apple.iTunes:DISCSUBTITLE`. Nomi per l'ordinamento da `TSOT`/`TSOP`/`TSOA`/`TSO2` (ID3), `TITLESORT`/`ARTISTSORT`/`ALBUMSORT`/`ALBUMARTISTSORT` (Vorbis), `sonm`/`soar`/`soal`/`soaa` (MP4). Generi multipli separati su `;` e `\0` (non sulla virgola). `compilation` da `TCMP`/`cpil`/`COMPILATION` = `1`.
  - `armonika_testdata::fixture::tagged_file(dir: &Path, name: &str, format: AudioFormat, tags: &FixtureTags) -> PathBuf`: copia il modello di silenzio da `bench/templates/` e scrive i tag con l'API nativa di lofty (split/merge, regole del §7.2). `FixtureTags` ha gli stessi campi di `TagFacts` più `cover: Option<Vec<u8>>`; `AudioFormat` qui è quello già esistente in `armonika-testdata` (`big.rs`), così `armonika-testdata` non dipende da `armonika-core`. Le crate lo usano solo come `dev-dependency`. Usato dai test di scanner, ricerca e Subsonic.

- [ ] **Step 1: Test che falliscono** in `tests/probe.rs`, ognuno su tutti i 7 formati (ciclo sui formati): `reads_basic_tags`, `reads_multi_valued_artists_and_mb_ids` (solo mp3, flac, ogg, opus, m4a: wav e aiff restano ID3 nel chunk e vanno provati, se lofty non li rilegge il test documenta il limite), `partial_dates_parsed` (`"1975"`, `"1975-10"`, `"1975-10-31"`), `original_date_separate_from_release_date` (remaster: `DATE=2011`, `ORIGINALDATE=1975`), `vinyl_track_number_kept_as_text` (`"A1"`: testo `A1`, numero `None`; `"3/12"`: testo `3`, numero 3, totale 12), `reads_sort_tags`, `reads_disc_subtitle`, `detects_embedded_cover`, `missing_tags_are_none`, `truncated_file_is_an_error_not_a_panic` (file troncato a metà), `extension_case_is_ignored` (`SONG.MP3`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** `probe.rs` e `fixture.rs`.
- [ ] **Step 4:** test verdi su Linux; sui tre sistemi in CI con la PR.
- [ ] **Step 5: Commit** `feat(media): read tags and audio properties without writing`

### Task 6: Accrediti e identità degli artisti

**Files:**
- Create: `crates/armonika-core/src/artists.rs`
- Test: `crates/armonika-core/tests/artists.rs`

**Interfaces:**
- Produces:
  - `split_credit(credit: &str, artists_tag: &[String], mbids: &[String]) -> Vec<CreditArtist>`, con `CreditArtist { credited_name: String, join_phrase: String, mbid: Option<String> }`, come le righe di `artist_credit_name` di MusicBrainz: `credited_name` è il nome come è scritto in questo accredito, `join_phrase` il testo che lo segue nell'accredito (`" feat. "`, `"; "`; vuoto per l'ultimo). Se i nomi vengono dal tag `ARTISTS`, `join_phrase` è il testo dell'accredito tra un nome e il successivo quando entrambi si trovano in ordine nell'accredito (confronto senza maiuscole), altrimenti vuoto. Concatenare `credited_name + join_phrase` di tutte le voci ridà l'accredito, quando i nomi vengono dal testo. Regole del §8.1 nell'ordine: (1) se `mbids` non è vuoto e `artists_tag` ha la stessa lunghezza, coppie nome+mbid; se `mbids` ha un solo valore, l'accredito intero con quel mbid; con lunghezze diverse e più di un mbid, si ignorano gli mbid; (2) se `artists_tag` non è vuoto, un artista per valore; (3) altrimenti divisione solo su `feat.`, `ft.`, `featuring` (parole intere, senza distinguere maiuscole, anche tra parentesi) e su `;`. Mai su `&`, `and`, `e`, `,`, `/`, `x`.
  - `resolve_artists(tx: &Transaction, library: LibraryId, list: &[CreditArtist], sort: Option<&str>) -> Result<Vec<ArtistId>>`: il chiamante salva poi in `track_artists` / `album_artists` artista, `credited_name` e `join_phrase` per posizione. `sort` (tag `ARTISTSORT` o `ALBUMARTISTSORT`) si salva in `artists.sort_name` solo se l'accredito ha un solo artista; ogni modifica di un artista aggiorna `updated_at`. Con mbid, cerca o crea l'artista per `mbid` (aggiorna il nome visualizzato se cambiato). Senza mbid: se tra gli artisti con mbid collegati a tracce o album della stessa libreria ce n'è **esattamente uno** con lo stesso `norm_name`, si usa quello (§8.7 punto 7); altrimenti si cerca o crea l'artista senza mbid con quel `norm_name`.

- [ ] **Step 1: Test che falliscono:** `ampersand_is_never_split` ("Simon & Garfunkel"), `comma_is_never_split` ("Earth, Wind & Fire"), `feat_is_split` ("Queen feat. David Bowie", "A (ft. B)", "A Featuring B"), `semicolon_is_split`, `artists_tag_wins_over_text`, `mbids_pair_with_artists_tag`, `single_mbid_applies_to_whole_credit`, `name_only_joins_unique_mbid_artist_in_library`, `name_only_stays_separate_with_homonyms` (due artisti con mbid diversi e lo stesso nome), `mbid_artist_in_other_library_does_not_capture_name_only`, `same_name_different_case_and_accents_is_one_artist` ("Beyoncé" e "beyonce": un artista, ma ogni traccia conserva il proprio `credited_name`), `join_phrases_rebuild_credit` ("A feat. B; C"), `join_phrase_from_artists_tag` (ARTISTS = [A, B], accredito "A & B": `join_phrase` di A è `" & "`), `sort_name_only_for_single_artist_credit`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(core): safe credit splitting and artist identity`

### Task 7: Scansione e collegamento file-traccia

**Files:**
- Create: `crates/armonika-scanner/` (Cargo.toml, `src/lib.rs`, `src/walk.rs`, `src/reconcile.rs`)
- Modify: `crates/armonika-core/src/model.rs` (eventi)
- Test: `crates/armonika-scanner/tests/scan.rs`

**Interfaces:**
- Consumes: `read_file` (Task 5), `resolve_artists` e `split_credit` (Task 6), `assign_album` (Task 8, nel frattempo un segnaposto che lascia `library_album_id` a NULL: Task 8 lo sostituisce).
- Produces:
  - In `model`: `LibraryChange { library: LibraryId, tracks_upserted: Vec<TrackId>, tracks_hidden: Vec<TrackId>, albums_touched: Vec<LibraryAlbumId>, artists_touched: Vec<ArtistId> }`.
  - `scan_library(db: &Db, library: LibraryId, opts: ScanOptions, progress: &dyn Fn(ScanProgress), changes: &dyn Fn(LibraryChange)) -> Result<ScanReport, ScanError>` (bloccante, chiamata dal controller in un thread). `ScanOptions { threads: usize, batch: usize /* default 500 */ }`. `ScanReport { seen, new, changed, unchanged, missing, reappeared, relinked_prefilter, unreadable, files_read, duration }`. `ScanError::RootUnavailable` se la radice non esiste, non è una cartella o non si legge.
  - Algoritmo (`reconcile.rs`):
    1. Verifica della radice; se fallisce, `RootUnavailable` e nessuna scrittura.
    2. Visita con `walkdir` (`follow_links(true)`; i cicli sono errori della singola voce, registrati e saltati), esclusa la cartella `.armonika` alla radice, solo estensioni accettate. Per ogni file: `stat` (dimensione, mtime in ms).
    3. Confronto con le tracce `present` della libreria per `rel_path` (mappa caricata all'inizio). Stessa dimensione, stesso mtime e `tag_schema == TAG_SCHEMA_VERSION`: invariato, nessuna lettura. Stessa dimensione e mtime ma `tag_schema` vecchio: si rileggono solo i tag (l'impronta del contenuto resta valida, perché il file non è cambiato). Diversi: si rilegge (`read_file`), si aggiornano le colonne, `hash_state='pending'`, `content_hash=NULL`.
    4. Percorso sconosciuto, in quest'ordine:
       a. **solo maiuscole cambiate** (Review Focus 3): se una traccia `present` della libreria ha lo stesso percorso confrontato senza maiuscole, stessa dimensione e stesso mtime, si aggiorna solo il suo `rel_path`, senza leggere il file;
       b. **file ricomparso**: se una traccia `missing` ha esattamente lo stesso `rel_path`, torna `present` (si rilegge il file solo se dimensione o mtime sono cambiati);
       c. **pre-filtro** (§11 punto 6): si legge il file con `read_file`; se c'è esattamente una traccia `missing` della stessa libreria con stessa dimensione, stesso mtime e durata entro 1 s, la traccia torna `present` sul nuovo percorso, con `prev_content_hash = content_hash`, `prev_rel_path = rel_path` vecchio, `content_hash=NULL`, `hash_state='pending'`;
       d. altrimenti nuova traccia con `TrackId::generate()`.
    5. Le letture con `read_file` girano in parallelo su `opts.threads` thread (pool `rayon` dedicato); le scritture avvengono in transazioni da `opts.batch` file, e dopo ogni transazione si emette `LibraryChange`, così le tracce compaiono durante la scansione (§11 punto 5).
    6. File illeggibili: contati in `unreadable`, registrati nei log, **nessuna** traccia creata o modificata; se il percorso apparteneva a una traccia `present`, la traccia resta com'era (sarà riletta alla prossima scansione perché mtime e dimensione in database restano i vecchi).
    7. A fine visita, le tracce `present` il cui percorso non è stato visto diventano `missing` con `missing_since`; si emette `LibraryChange` con `tracks_hidden`. Il pre-filtro del passo 4c vede quindi solo le tracce già mancanti da una scansione precedente o diventate mancanti in questa: per non perdere gli spostamenti avvenuti tra due scansioni, i file con percorso sconosciuto che non trovano candidati al passo 4c si rimettono in una lista e si riprovano dopo il passo 7, prima di creare tracce nuove.
  - `ScanReport` contiene anche `files_read` (quante volte è stata chiamata `read_file`), che il banco del Task 23 usa per verificare le scansioni incrementali. A fine scansione `serve` registra il report nei log come riga `scan finished` con i campi del report.

- [ ] **Step 1: Test che falliscono** (librerie create in `tempdir` con `fixture::tagged_file`): `first_scan_creates_tracks`, `rescan_without_changes_reads_nothing` (`unchanged == n`, `files_read == 0`), `tag_schema_bump_rereads_once` (tracce con `tag_schema = 0`: la prima scansione le rilegge e conserva `content_hash`, la seconda no), `move_between_scans_is_relinked` (spostamento fatto a server fermo: alla scansione successiva il percorso vecchio e quello nuovo si vedono nella stessa visita), `changed_tags_update_same_track` (stesso `TrackId`), `deleted_file_makes_track_missing_and_keeps_annotations` (preferito inserito prima resta), `reappeared_file_restores_same_track`, `moved_file_is_relinked_by_prefilter` (spostamento con `fs::rename` in un'altra cartella: stesso `TrackId`, `prev_content_hash` copiato), `two_missing_candidates_do_not_relink` (due copie identiche spostate: nascono tracce nuove), `case_only_rename_keeps_track`, `unavailable_root_aborts_without_marking_missing`, `unreadable_file_is_skipped_and_retried`, `dot_armonika_is_ignored`, `non_audio_files_are_ignored`, `changes_are_emitted_in_batches` (batch 2, 5 file: almeno 3 eventi), `symlink_loop_does_not_hang` (`#[cfg(unix)]`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(scanner): full and incremental scan with stable track identity`

### Task 8: Album della libreria

**Files:**
- Create: `crates/armonika-core/src/albums.rs`
- Modify: `crates/armonika-scanner/src/reconcile.rs`
- Test: `crates/armonika-core/tests/albums.rs`, `crates/armonika-scanner/tests/albums_scan.rs`

**Interfaces:**
- Produces:
  - `album_credit(track: &TagFacts, folder_has_mixed_artists: bool) -> String`: tag artista dell'album; se manca e `compilation`, `"Various Artists"`; se manca e `folder_has_mixed_artists` (decisione 2), `"Various Artists"`; altrimenti l'accredito della traccia.
  - `group_key(tags: &TagFacts, album_credit: &str) -> GroupKey`: `mb:<release_mbid>` se presente; altrimenti `tag:<normalize(album_title)>|<normalize(album_credit)>`; senza titolo dell'album, `singles:<ArtistId del primo artista dell'accredito>` (§8.7 punto 10, album della libreria "[Singles]" per artista e libreria; titolo mostrato `[Singles]`).
  - `assign_album(tx, library, &TrackId, &GroupKey, &TagFacts) -> Result<LibraryAlbumId>`: cerca un album della libreria con quella `group_key`, altrimenti lo crea; aggiorna titolo, accredito e artisti dell'album (`album_artists` con `split_credit` + `resolve_artists`).
  - **Regola della cartella** (§8.7 punto 10), applicata in `assign_album` prima della ricerca per chiave: una traccia senza `release_mbid` nella stessa cartella di tracce con lo stesso titolo e accredito dell'album che hanno un `release_mbid` (uno solo) entra in quell'album.
  - `reconcile_albums(tx, library) -> Result<Vec<LibraryAlbumId>>`, chiamata a fine scansione: preserva gli identificativi quando gli album si dividono o si uniscono per modifiche fatte da fuori (§8.1). Algoritmo:
    1. Per ogni album della libreria con tracce si calcola la chiave che le sue tracce hanno ora; un album le cui tracce hanno chiavi diverse si divide: la parte con più tracce tiene l'identificativo (a parità, la chiave minore in ordine lessicografico), le altre parti vanno all'album esistente con la loro chiave, o a un album nuovo.
    2. Se più album hanno la stessa chiave (unione), resta quello con più tracce (a parità, il più vecchio per `created_at`); le tracce degli altri passano a lui e le loro `album_annotations` si fondono (preferito se lo era in almeno uno, con la data più vecchia; valutazione più alta), poi per ogni album assorbito si scrive `add_redirect(vecchio public_id, public_id tenuto)` e l'album assorbito si cancella (le sue annotazioni sono già state fuse). Gli album rimasti senza tracce per altri motivi (tutte mancanti) restano, invisibili, con i loro dati personali.
    3. Campi dell'album calcolati dalle sue tracce `present`: data e data originale = il valore più frequente tra le tracce (a parità, il più vecchio); `sort_title` dal tag `ALBUMSORT` più frequente. Ogni modifica dell'album aggiorna `updated_at`.
    4. Restituisce gli album toccati, per la ricerca.
  - Visibilità: un album della libreria è visibile se ha almeno una traccia `present`.

- [ ] **Step 1: Test che falliscono:** `same_release_mbid_is_one_album`, `title_and_credit_group_without_mbid`, `multi_disc_folders_are_one_album` (`Disc 1/`, `Disc 2/`), `compilation_flag_gives_various_artists`, `mixed_artists_in_folder_give_various_artists`, `no_album_title_goes_to_artist_singles`, `folder_rule_joins_track_without_mbid`, `split_keeps_id_on_larger_part` (album da 8 tracce, 3 ritaggate con altro titolo: le 5 tengono l'id e il preferito), `merge_keeps_id_of_larger_and_moves_stars`, `merged_album_old_id_still_resolves` (`resolve_album` con l'id assorbito dà l'album tenuto), `empty_album_with_star_is_kept_hidden`, `album_original_date_from_tracks` (remaster 2011 di un album del 1975: data 2011, data originale 1975).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** e collegare allo scanner (Task 7 passo 5 per `assign_album`, fine scansione per `reconcile_albums`).
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(core): library album grouping with stable identity across splits and merges`

### Task 9: Impronta del contenuto in background e ricollegamento

**Files:**
- Create: `crates/armonika-media/src/content_hash.rs`, `crates/armonika-scanner/src/hasher.rs`
- Test: `crates/armonika-media/tests/content_hash.rs`, `crates/armonika-scanner/tests/hasher.rs`

**Interfaces:**
- Produces:
  - `content_hash(path: &Path) -> Result<[u8; 32], HashError>`: Symphonia (con `symphonia` per tutti i formati accettati; per Ogg/Opus basta il demuxer Ogg, nessuna decodifica), prima traccia audio, ogni pacchetto da `next_packet()` entra in BLAKE3 preceduto dalla sua lunghezza in 4 byte little-endian; hash intero di 32 byte (spike 5, variante A).
  - `Hasher::run(db: Db, stop: CancellationToken, changes: impl Fn(LibraryChange))`: un thread, bassa priorità; prende a lotti di 50 le tracce `present` con `hash_state='pending'`, calcola l'impronta, poi per ogni traccia:
    1. se `prev_content_hash` non è NULL (ricollegata dal pre-filtro): se coincide, si azzera `prev_content_hash`; se **non** coincide, il collegamento si annulla: la traccia torna `missing` con `rel_path = prev_rel_path` e `content_hash = prev_content_hash`, e per il file si crea una traccia nuova (stesse colonne del file e dei tag, nuovo `TrackId`, impronta appena calcolata, `hash_state='done'`). In entrambi i casi `prev_*` tornano NULL. Se la traccia non aveva un'impronta prima del ricollegamento, non c'è nulla da confrontare e il collegamento resta;
    2. altrimenti, se esattamente una traccia `missing` della stessa libreria ha la stessa impronta (con più candidate si preferisce lo stesso nome di file; se resta ambiguo, niente), la traccia mancante prende il file (percorso, colonne del file e dei tag, `state='present'`) e la traccia attuale si fonde in essa: annotazioni fuse con le regole del §8.7 punto 6 (preferito se lo era almeno una, valutazione più alta, ascolti sommati, `last_played_at` massimo), cronologia, voci di playlist e coda di riproduzione puntano alla traccia tenuta, poi si scrive `add_redirect(public_id della traccia attuale, public_id della traccia tenuta)` (un'app che ha già salvato l'identificativo nuovo, per esempio in un download offline, continua a trovarla) e la traccia attuale si cancella. La traccia tenuta è quella mancante, perché ha l'identificativo più vecchio e i dati personali. Ogni ricollegamento o annullamento emette `LibraryChange` (album ricalcolati con `assign_album`, indice aggiornato);
    3. errori di lettura: `hash_state='failed'`, nessun ricollegamento; un file con impronta fallita si riprova solo quando cambia (dimensione o mtime).
  - Il thread si mette in pausa mentre una scansione della stessa libreria è in corso (segnale dal controller, Task 10).

- [ ] **Step 1: Test che falliscono:** media: `hash_ignores_tag_changes` (stesso file, titolo cambiato con fixture: impronta uguale; tutti i formati tranne wav e aiff, per cui si documenta l'esito), `hash_differs_for_different_audio` (due modelli diversi), `hash_ignores_cover_change`. Hasher: `prefilter_link_confirmed_by_hash`, `prefilter_link_reverted_on_mismatch` (si simula sostituendo il contenuto audio con uno della stessa dimensione e mtime forzato con `filetime`), `moved_and_retagged_file_relinked_by_hash` (spostato e ritaggato: il pre-filtro non lo trova, l'hasher sì; preferiti e playlist della traccia vecchia restano sul `TrackId` vecchio), `ambiguous_hash_does_not_relink`, `merged_track_old_id_still_resolves`, `failed_hash_is_not_retried_until_file_changes`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(scanner): background content hash and relinking of moved files`

### Task 10: Controller della scansione, osservazione delle cartelle, scansione periodica

**Files:**
- Create: `crates/armonika-scanner/src/controller.rs`, `src/watch.rs`, `crates/armonika-server/src/cli/track.rs`
- Modify: `crates/armonika-server/src/serve.rs`
- Test: `crates/armonika-scanner/tests/controller.rs`, `crates/armonika-server/tests/cli_track.rs`

**Interfaces:**
- Produces:
  - `ScanController::new(db: Db, opts: ScanOptions, changes: mpsc::UnboundedSender<LibraryChange>) -> ScanController` (`Clone`); `request(&self, library: LibraryId)`: al massimo una scansione per libreria; richieste durante una scansione in corso si fondono in **una** scansione successiva. `request_all()`. `status(&self) -> ScanStatus { scanning: bool, libraries: Vec<(LibraryId, LibraryScanState { running, files_seen, last_report: Option<ScanReport>, last_error: Option<String> })> }`. Avvia anche l'`Hasher` (Task 9) e lo mette in pausa durante le scansioni.
  - `watch::spawn(controller, libraries: Vec<(LibraryId, PathBuf)>, interval: Duration) -> WatchHandle`: `notify` con `notify-debouncer-full` (2 s) per libreria; un evento sotto la radice (escluso `.armonika`) chiama `request(library)`. Se l'osservazione non si può avviare (es. condivisione di rete), si registra un `warn` e resta la scansione periodica. Timer: `request_all()` ogni `interval` (se diverso da zero). All'avvio di `serve`: `request_all()`.
  - CLI `armonika track purge-missing <library> [--older-than <durata>] --yes`: elimina definitivamente le tracce `missing` (con i dati personali collegati, per cascata); senza `--yes` stampa quante tracce e quante annotazioni, playlist e voci di cronologia verrebbero cancellate (§8.1: l'admin può eliminare le tracce mancanti). Si usa a server fermo o acceso (SQLite WAL).

- [ ] **Step 1: Test che falliscono:** `concurrent_requests_coalesce` (10 richieste durante una scansione lenta: al massimo 2 scansioni), `status_reports_running_scan`, `watcher_triggers_scan_on_new_file` (timeout 15 s), `periodic_scan_runs`, `hasher_pauses_during_scan`. CLI: `purge_missing_requires_yes`, `purge_missing_deletes_only_missing`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** e collegare in `serve` (controller, watcher; gli eventi `LibraryChange` per ora si scartano, li userà il Task 11).
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(scanner): scan controller, folder watching and periodic rescans`
- [ ] **Step 6: Pull request M2** `Phase 1 M2: scanner`.

---

## M3. Ricerca (ramo `phase-1/m3-search`)

### Task 11: `SearchIndex` su Tantivy

**Files:**
- Create: `crates/armonika-search/` (Cargo.toml, `src/lib.rs`, `src/tantivy_index.rs`, `src/cjk.rs`, `src/rerank.rs`, `src/updater.rs`)
- Modify: `crates/armonika-server/src/serve.rs`
- Test: `crates/armonika-search/tests/search.rs`, `tests/cjk.rs`

**Interfaces:**
- Produces:
  - `trait SearchIndex: Send + Sync { fn search(&self, q: &SearchQuery) -> Result<SearchHits>; fn upsert(&self, docs: Vec<SearchDoc>) -> Result<()>; fn remove(&self, ids: Vec<DocId>) -> Result<()>; fn clear(&self) -> Result<()>; }`.
  - `SearchQuery { text: String, libraries: Vec<LibraryId>, limits: GroupLimits { tracks, albums, artists: usize }, offsets: GroupLimits, boost: Option<Arc<dyn Fn(&DocId) -> f32 + Send + Sync>> }`. `SearchHits { tracks: Vec<TrackId>, albums: Vec<LibraryAlbumId>, artists: Vec<ArtistId> }`. `DocId { Track(TrackId), Album(LibraryAlbumId), Artist(ArtistId) }`. `SearchDoc { id: DocId, libraries: Vec<LibraryId>, primary: String, secondary: String, tertiary: String }` (traccia: titolo, accredito, titolo dell'album; album: titolo, accredito, vuoto; artista: nome, vuoto, vuoto; un artista appartiene alle librerie in cui ha tracce o album visibili).
  - `TantivyIndex::open(dir: &Path) -> Result<TantivyIndex>` (`data_dir/search/`), tre indici (tracce, album, artisti) come nello spike 1: campi testo con tokenizer `SimpleTokenizer` + `LowerCaser` + `AsciiFoldingFilter`; campo `library` (u64, indicizzato) per il filtro; per ogni parola della query `FuzzyTermQuery` (distanza 1 da 4 caratteri, 0 sotto; prefisso sull'ultima parola) su tutti i campi, parole in AND; più clausole `Should` con termini esatti, titolo x3. 50 candidati (o `offset + limit` se maggiore, al massimo 500), poi riordino.
  - `cjk.rs`: tokenizer `cjk_bigram` registrato in Tantivy: per ogni sequenza di caratteri CJK (Han, Hiragana, Katakana, Hangul) emette le coppie di caratteri consecutivi e, per le sequenze di un carattere, il carattere singolo. Ogni campo testo ha un campo gemello `*_cjk` con questo tokenizer. Una query con caratteri CJK aggiunge in AND, per ogni sequenza CJK della query, le sue coppie (o il carattere singolo) sui campi `*_cjk` (§11 punto 12).
  - `rerank.rs`: porting fedele di `score` e `rerank_group` da `spikes/01-search/src/common.rs`, più il segnale di popolarità: `costo_finale = costo - boost(id)`, dove il chiamante (Subsonic) passa `boost = 0.5 se preferito dall'utente + 0.25 * ln(1 + ascolti dell'utente)`, calcolato sui soli candidati. Parità: ordine del motore.
  - `updater::run(db: Db, index: Arc<dyn SearchIndex>, rx: mpsc::UnboundedReceiver<LibraryChange>)`: legge dal database i documenti toccati (solo tracce `present` e album e artisti visibili), `upsert` e `remove` a lotti, commit al massimo ogni 2 s. `updater::rebuild(db, index)`: `clear` e reindicizzazione completa; si esegue all'avvio se l'indice manca o se la sua versione di schema (costante `INDEX_SCHEMA_VERSION`, salvata in `settings`) è diversa. L'`IndexWriter` si crea per ogni commit (heap 15 MB, 1 thread) e si chiude subito, per la RAM a riposo (§3).

- [ ] **Step 1: Test che falliscono** (indice su `tempdir`, documenti costruiti a mano): `finds_with_one_typo` ("bohemain" trova "Bohemian Rhapsody"), `finds_without_accents` ("beyonce" trova "Beyoncé", "sigur ros" trova "Sigur Rós"), `prefix_on_last_word` ("bohemian rhap"), `multiword_is_and`, `title_beats_album_match` (la traccia con la parola nel titolo prima di quella con la parola nell'album), `library_filter_hides_other_libraries`, `boost_reorders_ties`, `offset_pages_results`, `removed_doc_disappears`. CJK: `cjk_substring_found` ("龍一" trova "坂本龍一"), `single_cjk_char_found`, `cjk_mixed_with_latin` ("YOASOBI 夜に" trova "夜に駆ける" di YOASOBI). Updater: `missing_track_leaves_index`, `rebuild_matches_database`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** e collegare in `serve` (canale dal `ScanController` all'updater).
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(search): tantivy search index with typo tolerance, accent folding and CJK bigrams`
- [ ] **Step 6: Pull request M3** `Phase 1 M3: search`.

---

## M4. API OpenSubsonic (ramo `phase-1/m4-subsonic`)

Riferimento: https://opensubsonic.netlify.app/docs/ (endpoint, parametri, campi). Versione dichiarata `1.16.1`, `type: "armonika"`, `serverVersion: VERSION`, `openSubsonic: true`. Gli endpoint sono quelli che usano Symfonium e Feishin (§12); quelli non elencati rispondono errore 0 "not implemented" con HTTP 200, come vuole il protocollo.

### Task 12: Router, autenticazione, risposte e endpoint di sistema

**Files:**
- Create: `crates/armonika-subsonic/` (Cargo.toml, `src/lib.rs`, `src/auth.rs`, `src/response.rs`, `src/error.rs`, `src/system.rs`)
- Modify: `crates/armonika-server/src/serve.rs`
- Test: `crates/armonika-subsonic/tests/common/mod.rs`, `tests/auth.rs`, `tests/system.rs`

**Interfaces:**
- Produces:
  - `AppState { db: Db, secrets: Arc<SecretBox>, search: Arc<dyn SearchIndex>, scans: ScanController, media: MediaConfig { ffmpeg: Option<PathBuf>, transcode_format, transcode_bitrate_kbps, cover_cache_dir }, scrobbles: ScrobbleSink /* Task 19; nel frattempo no-op */, ignored_articles: Vec<String> }`.
  - `router(state: AppState) -> axum::Router`: `/rest/{method}` e `/rest/{method}.view`, GET e POST (`application/x-www-form-urlencoded`, estensione `formPost`); parametri ripetuti (`id=a&id=b`) letti come lista.
  - Autenticazione (`auth.rs`), estrattore `Caller(Access)`: `u` + `t` + `s` (token = md5(password Subsonic + sale), confronto in tempo costante); `u` + `p` in chiaro o `enc:<hex>`; `apiKey` (estensione `apiKeyAuthentication`). Errori: 10 parametro mancante, 40 credenziali errate, 43 `apiKey` insieme a `u`/`p`/`t`, 44 API key non valida, 50 non autorizzato. Ritardo fisso di 500 ms sulle risposte 40 e 44.
  - Risposte (`response.rs`): gli handler restituiscono `Payload(&'static str /* nome del nodo, es. "album" */, serde_json::Value)`; `f=json` produce `{"subsonic-response": {status, version, type, serverVersion, openSubsonic, <nodo>: ...}}`; `f=xml` (default) converte il `Value` con regole fisse: scalari come attributi, oggetti come elementi figli, array come elementi ripetuti con il nome della chiave, chiave `value` come testo dell'elemento; namespace `http://subsonic.org/restapi`. `f=jsonp` non supportato (errore 0).
  - Ogni risposta `/rest` ha l'intestazione `Server-Timing: app;dur=<ms>` (tempo lato server, usato dal banco del Task 23).
  - Visibilità: ogni handler usa `Access::visible(musicFolderId)`; un id di una traccia, album o artista non visibile risponde **70 not found**, mai 50 (non si rivela l'esistenza). Ogni id ricevuto passa da `resolve_*` (Task 2), quindi un id assorbito in un'unione porta all'entità tenuta; le risposte riportano sempre l'id tenuto.
  - Endpoint: `ping`, `getLicense` (`valid: true`), `getOpenSubsonicExtensions` (`formPost` v1, `apiKeyAuthentication` v1), `getMusicFolders` (solo le librerie visibili), `getUser` (solo se stesso, o qualsiasi per un admin; `streamRole`, `downloadRole = can_download`, `adminRole`, `scrobblingEnabled`, `folder` = librerie visibili), `getScanStatus` (`scanning`, `count` = file visti), `startScan` (solo chi ha `manage` su almeno una libreria: avvia la scansione delle librerie che gestisce).

- [ ] **Step 1: Test che falliscono** (con un helper `common::app()` che crea database, utenti `admin` e `ann` (accesso `listen` alla libreria A) e una libreria con 3 tracce di fixture, e chiama il router con `oneshot`): `token_auth_ok`, `plain_and_enc_password_ok`, `api_key_ok`, `wrong_password_is_40`, `api_key_with_user_is_43`, `missing_user_is_10`, `json_and_xml_envelopes` (stesso contenuto, XML valido con il namespace), `view_suffix_and_post_form_work`, `music_folders_only_visible`, `get_user_reports_download_role`, `start_scan_requires_manage`, `unknown_method_is_error_0`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** e montare il router in `serve`.
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(subsonic): router, authentication, json and xml responses, system endpoints`

### Task 13: Navigazione

**Files:**
- Create: `crates/armonika-core/src/tracks.rs`, `crates/armonika-subsonic/src/browse.rs`
- Test: `crates/armonika-subsonic/tests/browse.rs`

**Interfaces:**
- Produces:
  - `tracks.rs` (core): query di sola lettura che applicano sempre `state='present'` e il filtro delle librerie: `track(conn, &[LibraryId], &TrackId) -> Result<Option<TrackView>>`, `album(conn, libs, &LibraryAlbumId) -> Result<Option<(AlbumView, Vec<TrackView>)>>` (tracce in ordine disco, poi `track_no`; se manca `track_no`, ordine naturale di `track_number_text` con lettere e numeri separati, così `A1, A2, A10, B1`; poi titolo), `artist(conn, libs, &ArtistId) -> Result<Option<(ArtistView, Vec<AlbumView>)>>` (album di cui è artista dell'album), `artists(conn, libs) -> Vec<ArtistView>`, `album_list(conn, libs, AlbumListKind, size, offset, user: &UserId) -> Vec<AlbumView>`, `random_tracks(conn, libs, size, genre, from_year, to_year)`, `tracks_by_genre`, `genres(conn, libs) -> Vec<(String, song_count, album_count)>`, `all_tracks_page(conn, libs, offset, limit)` (ordine per `id`, per il Task 14). `TrackView`, `AlbumView`, `ArtistView` contengono i campi Subsonic più le annotazioni dell'utente (`starred`, `userRating`, `playCount`, `played`).
  - `AlbumListKind { Random, Newest, Frequent, Recent, Starred, AlphabeticalByName, AlphabeticalByArtist, ByYear { from, to }, ByGenre(String), Highest }`; `Frequent`, `Recent`, `Starred`, `Highest` usano i dati **dell'utente che chiama**.
  - Ordinamento per nome di artisti, album e tracce: `sort_name` / `sort_title` se presenti, altrimenti `fold_sort` con gli articoli della configurazione. `AlphabeticalByName` e `ByYear` usano la data **originale** se c'è, altrimenti quella dell'edizione.
  - Endpoint: `getArtists` (indice per iniziale con `ignoredArticles` dalla configurazione; `ifModifiedSince` e `lastModified` di `getIndexes` dal massimo `updated_at` di tracce, album e artisti visibili), `getArtist`, `getAlbum`, `getSong`, `getGenres`, `getAlbumList2`, `getAlbumList` (stessi dati, nodo `albumList`), `getRandomSongs`, `getSongsByGenre`, `getIndexes` e `getMusicDirectory` (vista a cartelle simulata: artisti, poi album della libreria, poi tracce, come Navidrome), `getArtistInfo2` e `getAlbumInfo2` (risposta vuota valida, nessun servizio esterno in questa fase), `getTopSongs` e `getSimilarSongs2` (liste vuote).
  - Campi di una traccia (`child`): `id`, `parent` (album), `title`, `album`, `artist`, `track` (`track_no`, assente se è `None`), `discNumber`, `year` (anno dell'edizione), `genre`, `sortName`, `displayArtist` (accredito come scritto nel file), `coverArt` (= id dell'album se l'album ha una copertina, altrimenti id della traccia), `size`, `contentType`, `suffix`, `duration` (s), `bitRate`, `samplingRate`, `channelCount`, `bitDepth`, `path` (`rel_path`), `isDir: false`, `type: "music"`, `albumId`, `artistId` (primo artista), `artists` e `albumArtists` (OpenSubsonic, liste `{id, name}`), `musicBrainzId` (registrazione), `created` (`added_at`), `starred`, `userRating`, `playCount`, `played`. Gli elementi di `artists` e `albumArtists` usano il nome dell'artista; l'accredito com'era scritto resta in `artist` e `displayArtist`.
  - Campi aggiuntivi di un album (OpenSubsonic): `releaseDate` e `originalReleaseDate` come `{year, month, day}` con i campi assenti omessi, `year` = anno originale se c'è, altrimenti dell'edizione (così un remaster si ordina con l'album originale), `sortName`, `discTitles` (`[{disc, title}]` da `disc_subtitle`), `isCompilation`, `musicBrainzId` (edizione).

- [ ] **Step 1: Test che falliscono:** `artists_indexed_ignoring_articles` ("The Beatles" sotto B), `album_tracks_in_disc_track_order`, `missing_track_not_listed_and_get_song_is_70`, `other_library_album_is_70`, `music_folder_filter_applies`, `album_list_starred_is_per_user` (preferito di `admin` non compare per `ann`), `album_list_paging_is_stable`, `random_songs_respects_size`, `directory_browsing_walks_artist_album_tracks`, `multi_artist_track_appears_under_each_artist` (§ Glossario, accredito "A feat. B"), `vinyl_sides_in_natural_order` (A1, A2, A10, B1), `sort_tag_orders_artist` ("David Bowie" con `ARTISTSORT` "Bowie, David" sotto B), `remaster_sorted_by_original_year`, `album_exposes_release_and_original_dates`, `merged_album_old_id_returns_kept_album`, `indexes_if_modified_since_returns_empty_when_unchanged`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(subsonic): browsing endpoints with per-user visibility`

### Task 14: Ricerca Subsonic

**Files:**
- Create: `crates/armonika-subsonic/src/search.rs`
- Test: `crates/armonika-subsonic/tests/search.rs`

**Interfaces:**
- Consumes: `SearchIndex::search` (Task 11), `tracks::all_tracks_page` e le viste (Task 13).
- Produces: `search3` e `search2` (stessi dati, nodo `searchResult2`): parametri `query`, `artistCount`/`albumCount`/`songCount` (default 20, massimo 500), `*Offset`, `musicFolderId`. Query vuota, solo spazi o `""` (due virgolette): niente indice, si restituiscono tutte le tracce, gli album e gli artisti visibili a pagine, in ordine stabile per `id` (Review Focus 2). Altrimenti `SearchIndex::search` con `boost` costruito dalle annotazioni dell'utente sui candidati.

- [ ] **Step 1: Test che falliscono:** `typo_query_finds_track`, `empty_query_pages_whole_library` (1.200 tracce di fixture solo nel database, pagine da 500: 1.200 id distinti, nessun duplicato, poi pagina vuota), `quoted_empty_query_is_empty_query`, `search_respects_library_access`, `own_star_boosts_result`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(subsonic): search3 and search2 with full-library paging`

### Task 15: Streaming, download e copertine

**Files:**
- Create: `crates/armonika-media/src/cover.rs`, `crates/armonika-subsonic/src/media.rs`
- Test: `crates/armonika-media/tests/cover.rs`, `crates/armonika-subsonic/tests/media.rs`

**Interfaces:**
- Produces:
  - `cover::embedded(path: &Path) -> Result<Option<Image { bytes: Vec<u8>, mime: String }>>` (lofty, preferenza `CoverFront`, poi la prima immagine); `cover::folder(dir: &Path) -> Option<PathBuf>` (primo esistente tra `cover`, `folder`, `front`, `album` con estensione `jpg`, `jpeg`, `png`, `webp`, senza distinguere maiuscole); `cover::resize(bytes: &[u8], size: u32) -> Result<Vec<u8>>` (crate `image`, lato lungo `size`, JPEG qualità 85, mai ingrandire).
  - `stream`: risolve la traccia visibile, percorso = radice della libreria + `rel_path`; senza transcodifica serve il file con `tower_http::services::ServeFile` (range HTTP, `Content-Type` dal formato, `Accept-Ranges: bytes`). File sparito dal disco: errore 70 e la traccia si segnala allo `ScanController` (`request(library)`).
  - `download`: come `stream` senza transcodifica, solo se `can_download` (altrimenti errore 50), con `Content-Disposition: attachment` e il nome del file.
  - `getCoverArt(id, size)`: `id` di album (copertina di cartella della prima traccia, altrimenti incorporata della prima traccia) o di traccia (incorporata, altrimenti di cartella). Cache in `data_dir/cache/covers/<id>-<size|orig>-<mtime della sorgente>.jpg`; nessuna copertina: errore 70.

- [ ] **Step 1: Test che falliscono:** media: `embedded_front_cover_preferred`, `folder_cover_case_insensitive`, `resize_never_upscales`. Subsonic: `stream_full_file_matches_bytes`, `stream_range_returns_206_and_slice` (`Range: bytes=100-199`), `stream_other_library_is_70`, `download_requires_download_role`, `cover_for_album_uses_folder_image`, `cover_resized_and_cached` (seconda richiesta non rilegge la sorgente: si controlla il file in cache), `stream_of_vanished_file_requests_rescan`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(subsonic): streaming with http ranges, download role and cover art`

### Task 16: Transcodifica con ffmpeg

**Files:**
- Create: `crates/armonika-media/src/transcode.rs`
- Modify: `crates/armonika-subsonic/src/media.rs`
- Test: `crates/armonika-media/tests/transcode.rs`, `crates/armonika-subsonic/tests/transcode.rs`

**Interfaces:**
- Produces:
  - `transcode::plan(source_format: AudioFormat, source_kbps: Option<u32>, requested_format: Option<&str>, max_kbps: Option<u32>, defaults: (&str, u32)) -> TranscodePlan { Raw, Encode { format: TargetFormat /* Mp3 | Opus | Aac */, kbps: u32 } }`. Regole: `format=raw`, oppure nessun `format` e nessun `maxBitRate` (o `maxBitRate=0`), oppure `maxBitRate` maggiore o uguale al bitrate sorgente e nessun `format` diverso dalla sorgente: `Raw`. Altrimenti `Encode` con il formato richiesto (se tra `mp3`, `opus`, `aac`, altrimenti quello di default della configurazione) e `kbps = min(maxBitRate, default)` se `maxBitRate` c'è, altrimenti il default.
  - `transcode::spawn(ffmpeg: &Path, input: &Path, plan, time_offset_s: Option<u32>) -> Result<impl Stream<Item = io::Result<Bytes>>>`: `ffmpeg -v error -ss <offset> -i <input> -map 0:a:0 -vn` più codec (`libmp3lame`, `libopus` in contenitore ogg, `aac` in contenitore `adts`) e `-b:a <kbps>k -f <fmt> pipe:1`; il processo si uccide quando il client chiude la connessione.
  - `stream` con `Encode` e ffmpeg assente: si serve il file originale e si registra un `warn` una volta sola. Risposta transcodificata senza `Content-Length` e senza range.

- [ ] **Step 1: Test che falliscono:** `plan_raw_when_no_params`, `plan_raw_when_max_above_source`, `plan_encode_with_max_below_source`, `plan_unknown_format_uses_default`. Con ffmpeg (test saltati se `ffmpeg` non è nel `PATH`, ma in CI c'è): `mp3_transcode_produces_decodable_audio` (l'output letto da `probe` su file temporaneo), `client_disconnect_kills_ffmpeg`. Subsonic: `stream_with_max_bitrate_is_transcoded`, `missing_ffmpeg_falls_back_to_raw`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(media): on-the-fly transcoding with external ffmpeg`

### Task 17: Preferiti, valutazioni, ascolti e "in ascolto ora"

**Files:**
- Create: `crates/armonika-core/src/annotations.rs`, `crates/armonika-subsonic/src/annotate.rs`
- Modify: `crates/armonika-core/src/play_queue.rs` (solo `now_playing`)
- Test: `crates/armonika-core/tests/annotations.rs`, `crates/armonika-subsonic/tests/annotate.rs`

**Interfaces:**
- Produces:
  - `annotations::star(conn, &UserId, &[DocId])`, `unstar`, `set_rating(conn, &UserId, &DocId, rating: u8 /* 0 = togli */)`, `record_play(conn, &UserId, &TrackId, at_ms) -> Result<()>` (incrementa `play_count`, aggiorna `last_played_at`, aggiunge a `play_history`), `starred(conn, &UserId, libs) -> (Vec<ArtistView>, Vec<AlbumView>, Vec<TrackView>)`.
  - `play_queue::set_now_playing(conn, &UserId, client: &str, &TrackId)`, `now_playing(conn, viewer: &Access) -> Vec<NowPlayingEntry>`: il proprio ascolto sempre; quello degli altri solo se hanno `share_now_playing` e la traccia è visibile a chi guarda; voci più vecchie di 10 minuti escluse.
  - Endpoint: `star`, `unstar` (`id`, `albumId`, `artistId`, anche ripetuti), `setRating`, `scrobble` (`id` ripetibile, `time` in ms, `submission` default `true`): con `submission=true` `record_play` e accodamento allo scrobbling (Task 19); con `false` `set_now_playing` e "in ascolto ora" verso i servizi; `getStarred2`, `getStarred`, `getNowPlaying`.
  - Un id non visibile in `star` o `scrobble` risponde 70 e non scrive nulla.

- [ ] **Step 1: Test che falliscono:** `star_track_album_artist_and_list`, `stars_are_per_user`, `rating_zero_removes_rating`, `scrobble_submission_counts_play`, `scrobble_now_playing_does_not_count`, `now_playing_private_by_default`, `now_playing_shared_when_enabled`, `annotations_survive_access_loss_and_return` (§8.5: si toglie l'accesso, `getStarred2` non mostra la traccia, si ridà l'accesso e torna), `star_invisible_track_is_70`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(subsonic): stars, ratings, play counts and now playing`

### Task 18: Playlist e coda di riproduzione salvata

**Files:**
- Create: `crates/armonika-core/src/playlists.rs`, `crates/armonika-subsonic/src/playlists.rs`
- Modify: `crates/armonika-core/src/play_queue.rs`
- Test: `crates/armonika-core/tests/playlists.rs`, `crates/armonika-subsonic/tests/playlists.rs`

**Interfaces:**
- Produces:
  - `playlists::create(conn, owner, name, tracks: &[TrackId]) -> PlaylistId`, `get(conn, viewer: &Access, &PlaylistId) -> Option<(PlaylistView, Vec<TrackView>)>` (solo voci visibili, nell'ordine), `list(conn, viewer) -> Vec<PlaylistView>` (proprie e pubbliche degli altri), `update(conn, editor: &Access, &PlaylistId, PlaylistEdit { name, comment, public, add: Vec<TrackId>, remove_visible_indexes: Vec<usize> })`, `replace_visible(conn, editor, &PlaylistId, new_visible: &[TrackId])`, `delete`. Solo il proprietario modifica o cancella (altrimenti errore 50); una playlist pubblica la vedono tutti, in sola lettura.
  - Voci nascoste (§8.7 punto 9): per chi chiama, una voce è nascosta se la traccia non è `present` o è di una libreria non visibile. Gli indici delle API si riferiscono sempre alla lista visibile. `add` aggiunge in fondo alla lista completa. `remove_visible_indexes` toglie solo voci visibili. Algoritmo di `replace_visible` (non determinato dai test, va seguito così):
    1. La lista completa L si divide in "corse" di voci nascoste consecutive; ogni corsa è ancorata alla voce visibile che la precede in L, oppure all'inizio.
    2. Le voci visibili vecchie si abbinano a quelle nuove per (traccia, numero di occorrenza di quella traccia): la seconda occorrenza di T nella lista vecchia con la seconda nella nuova.
    3. La lista risultante: prima la corsa ancorata all'inizio; poi, per ogni voce nuova, la voce seguita dalla corsa della voce vecchia abbinata, se c'è.
    4. Una corsa la cui voce vecchia non è abbinata (tolta) passa alla voce vecchia abbinata più vicina che la precedeva in L (o all'inizio), dopo la corsa di quella.
  - `play_queue::save(conn, &UserId, tracks: &[TrackId], current: Option<usize>, position_ms, client)`, `load(conn, viewer) -> Option<PlayQueueView>` (solo voci visibili; `current` ricalcolato sulla lista visibile; vale l'ultimo salvataggio, §8.5).
  - Endpoint: `getPlaylists`, `getPlaylist`, `createPlaylist` (con `playlistId` è la sostituzione completa: `replace_visible`), `updatePlaylist` (`songIdToAdd`, `songIndexToRemove`, `name`, `comment`, `public`), `deletePlaylist`, `savePlayQueue`, `getPlayQueue`.

- [ ] **Step 1: Test che falliscono** (core): `public_playlist_is_read_only_for_others`, `private_playlist_invisible_to_others` (70), `same_track_twice_allowed`, `hidden_entries_not_returned_but_kept`, `remove_by_visible_index_keeps_hidden` (L = [A, h1, B, h2, C], h nascoste; togliere l'indice visibile 1 (B) dà [A, h1, h2, C]), `replace_keeps_hidden_after_anchor` (L = [A, h1, B, C], nuova visibile [C, A, B] dà [C, A, h1, B]), `replace_moves_orphan_run_back` (L = [A, B, h1, C], nuova [A, C] dà [A, h1, C]), `replace_with_duplicates_matches_occurrences` (L = [A, h1, A, h2], nuova [A, A] dà [A, h1, A, h2]), `missing_track_hidden_then_visible_again`, `play_queue_last_save_wins`, `play_queue_hides_invisible_and_recomputes_current`. Subsonic: `create_update_get_playlist_roundtrip`, `update_by_other_user_is_50`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare.**
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(subsonic): playlists with hidden entries and saved play queue`
- [ ] **Step 6: Pull request M4** `Phase 1 M4: OpenSubsonic API`.

---

## M5. Scrobbling (ramo `phase-1/m5-scrobble`)

### Task 19: ListenBrainz

**Files:**
- Create: `crates/armonika-scrobble/` (Cargo.toml, `src/lib.rs`, `src/outbox.rs`, `src/listenbrainz.rs`)
- Modify: `crates/armonika-subsonic/src/annotate.rs`, `crates/armonika-server/src/serve.rs`, `crates/armonika-server/src/cli/user.rs`
- Test: `crates/armonika-scrobble/tests/listenbrainz.rs`, `tests/outbox.rs`

**Interfaces:**
- Produces:
  - `ScrobbleSink` (`Clone`): `now_playing(&UserId, &TrackId)` (best effort, nessun nuovo tentativo) e `submitted(&UserId, &TrackId, played_at_ms)` (scrive una riga in `scrobble_outbox` per ogni servizio collegato all'utente, in fase 1 solo ListenBrainz, nella stessa transazione di `record_play`).
  - `outbox::run(db, sb, services: Vec<Arc<dyn ScrobbleService>>, stop)`: invia le righe scadute a lotti (ListenBrainz fino a 100 ascolti). `ScrobbleService` è un trait con `name()`, `now_playing(..)` e `submit(.., batch) -> Result<(), SubmitError { Retry, Auth, Rejected }>`, così Last.fm si aggiunge più avanti come seconda implementazione; in caso di errore di rete o 5xx, nuovo tentativo con attesa esponenziale da 1 min a 6 h; errore di autenticazione: la riga resta e si registra un `warn` per utente una sola volta; dopo 14 giorni una riga si scarta.
  - Dati inviati (Glossario): titolo, accredito, album, durata, numero di traccia dai tag della traccia; `recording_mbid` (`additional_info.recording_mbid`) se noto. `listened_at` in secondi.
  - ListenBrainz: `POST {url}/1/submit-listens` (`listen_type` `single`/`import` e `playing_now`), intestazione `Authorization: Token <token>`; `validate_token` con `GET /1/validate-token`.
  - CLI: `armonika user listenbrainz <name> --token-stdin` (verifica il token e lo salva cifrato), `armonika user scrobble-unlink <name> listenbrainz`.

- [ ] **Step 1: Test che falliscono** (server finti con `wiremock`): `listenbrainz_submit_payload_shape` (con `recording_mbid`), `listenbrainz_now_playing`, `listenbrainz_batch_of_100`, `outbox_retries_after_5xx`, `outbox_keeps_rows_on_auth_error`, `outbox_drops_after_14_days`, `no_linked_service_writes_no_rows`, `tokens_are_stored_encrypted`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare** e collegare a `scrobble` (Task 17).
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(scrobble): listenbrainz with a persistent outbox`
- [ ] **Step 6: Pull request M5** `Phase 1 M5: scrobbling`.

---

## M6. Consegna e misure (ramo `phase-1/m6-delivery`)

### Task 20: Avvio completo e modalità headless

**Files:**
- Modify: `crates/armonika-server/src/serve.rs`, `README.md`
- Create: `docs/configuration.md`
- Test: `crates/armonika-server/tests/serve_e2e.rs`

**Interfaces:**
- Produces: ordine di avvio di `serve`: configurazione, `data_dir`, `Db::open`, `SecretBox`, `bootstrap_admin`, `sync_from_config`, indice (con `rebuild` se serve, in background: le API rispondono subito, la ricerca è vuota finché non finisce), `ScanController` + `Hasher` + watcher, outbox degli scrobble, router (`/rest` sempre; `/` solo con `web.enabled`). Arresto: si fermano watcher, scansioni (fine del lotto corrente), hasher e outbox, poi il server HTTP con un'attesa massima di 10 s.

- [ ] **Step 1: Test che falliscono** (processo vero con `assert_cmd`, libreria di fixture, admin da variabili d'ambiente): `e2e_scan_search_stream` (attende che `getScanStatus` dica `scanning=false`, `search3` trova una traccia, `stream` restituisce i byte del file), `headless_disables_root_but_keeps_api`, `restart_keeps_ids_and_stars`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare**; scrivere `docs/configuration.md` (tutte le chiavi del Task 1 con default e variabile d'ambiente, avviso su `inbox` non ancora attiva, nota su chi usa Armonika per un servizio commerciale, §7.2) e aggiornare il README con "come provarlo".
- [ ] **Step 4:** test verdi.
- [ ] **Step 5: Commit** `feat(server): full startup wiring, graceful shutdown and headless mode`

### Task 21: Immagine Docker

**Files:**
- Create: `Dockerfile`, `.dockerignore`, `docker-compose.example.yml`, `.github/workflows/docker.yml`
- Modify: `.github/workflows/ci.yml` (nessun cambiamento ai job esistenti; solo il filtro dei percorsi se serve)

**Interfaces:**
- Produces:
  - `Dockerfile` multi-stadio: compilazione con `rust:1.99-slim-bookworm` (`cargo build --release --locked -p armonika-server`), esecuzione su `debian:bookworm-slim` con `ffmpeg` e `ca-certificates`, utente non root con uid 1000 (chi ha la musica con un altro proprietario usa `user:` nel compose, documentato nell'esempio), `ENV ARMONIKA_DATA_DIR=/data ARMONIKA_SERVER__BIND=0.0.0.0:4747`, `VOLUME /data`, `EXPOSE 4747`, `HEALTHCHECK` con `armonika healthcheck` (nuovo sottocomando che chiama `GET /rest/ping` senza autenticazione e accetta la risposta di errore 10 come "vivo").
  - `docker-compose.example.yml`: servizio `armonika`, volumi `./data:/data` e `/percorso/della/musica:/music:ro`, variabili `ARMONIKA_ADMIN_USER` e `ARMONIKA_ADMIN_PASSWORD`, libreria `[[library]] name="Music" path="/music"` passata con `ARMONIKA_CONFIG` o con un file `armonika.toml` montato.
  - Workflow `docker.yml`: su push a `main` e sui tag `v*`, `docker/setup-qemu-action`, `docker/setup-buildx-action`, build `linux/amd64,linux/arm64`, push su `ghcr.io/filocorsa/armonika` con tag `main` e `sha-<short>` (e la versione sui tag). Sulle pull request solo build `linux/amd64` senza push, e solo se cambiano `Dockerfile`, `Cargo.lock` o `crates/`.

- [ ] **Step 1:** costruire in locale `docker build -t armonika:dev .`. Atteso: immagine creata.
- [ ] **Step 2:** `docker compose -f docker-compose.example.yml up` con una cartella di fixture come musica: `docker inspect` mostra lo stato `healthy` entro 60 s e `curl "http://localhost:4747/rest/ping?u=...&p=...&f=json"` risponde `"status":"ok"`.
- [ ] **Step 3:** il container gira con `/music` in sola lettura senza errori (nessuna scrittura nelle librerie, Global Constraints).
- [ ] **Step 4: Commit** `build: docker image with ffmpeg and multi-arch publishing to ghcr`

### Task 22: Dati per i benchmark

**Files:**
- Modify: `bench/testdata/src/names.rs`, `bench/testdata/src/big.rs`, `bench/testdata/tests/big.rs`, `docs/development.md`

**Interfaces:**
- Produces:
  - Vocabolario di `gen-big` allargato: pseudo-parole da sillabe con frequenza di tipo Zipf (porting da `spikes/01-search/src/realistic.rs`), mantenendo nomi accentati, CJK e caratteri vietati nei nomi di file. Obiettivo: almeno 20.000 parole distinte su 100.000 tracce e almeno il 90% dei titoli unici.
  - `manifest.json` invariato nel formato; il seed continua a dare un manifest identico byte per byte.

- [ ] **Step 1: Test che falliscono** in `tests/big.rs`: `vocabulary_is_large` (su 20.000 tracce: almeno 10.000 parole distinte; misura su 100.000 riportata in `docs/development.md`), `readback_checks_all_written_fields` (estende il test di rilettura esistente: oltre a titolo e artista anche album, numero di traccia, numero di disco e anno).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implementare**; aggiornare i numeri in `docs/development.md`.
- [ ] **Step 4:** test verdi; i test esistenti del generatore (`same_seed_gives_identical_manifest`, percorsi sicuri, collisioni) restano verdi.
- [ ] **Step 5: Commit** `bench: larger vocabulary for the fake library and full tag read-back`

### Task 23: Banco di misura delle soglie del §3

**Files:**
- Create: `bench/harness/Cargo.toml`, `bench/harness/src/main.rs`, `docs/benchmarks.md`
- Modify: `Cargo.toml` (members), `.github/workflows/ci.yml` (job `bench-smoke`)

**Interfaces:**
- Produces: binario `armonika-bench` (crate `armonika-bench`, `publish = false`):
  - `armonika-bench run --library <dir> --manifest <manifest.json> --binary <path di armonika> [--queries 1000] [--out results.json]`: avvia `armonika serve` su una `data_dir` vuota con la libreria data, misura:
    1. **prima scansione**: tempo fino a `scanning=false` e numero di tracce (deve essere uguale alle voci del manifest; il confronto dei percorsi normalizza le cartelle che differiscono solo per maiuscole, perché su NTFS e APFS le collisioni finiscono nella prima cartella creata);
    2. **seconda scansione** senza cambiamenti (`startScan`): tempo e `files_read` letto dalla riga `scan finished` nei log del processo (Task 7): deve essere 0;
    3. **ricerca**: query generate dal manifest (stesso schema dello spike 1: `prefix`, `typo`, `diacritics`, `multiword`, seed fisso) via HTTP `search3` con `f=json`; p50 e p95 per tipo, misurati con l'intestazione `Server-Timing` che il server aggiunge (`app;dur=<ms>`, Task 12 la aggiunge a tutte le risposte `/rest`) e, a parte, il tempo totale lato client;
    4. **primo byte** di `stream` senza transcodifica su 200 tracce a caso: p50 e p95;
    5. **RAM a riposo** del processo dopo 60 s senza richieste (crate `sysinfo`).
  - Uscita: tabella leggibile più JSON; codice di uscita 1 se una soglia rigida è superata (ricerca p95 >= 50 ms, primo byte p95 >= 200 ms, seconda scansione con file riletti). Prima scansione e RAM sono indicative (§3): si riportano senza far fallire.
  - CI `bench-smoke` (Linux, a ogni push con codice): genera 5.000 tracce con `gen-big`, esegue `armonika-bench run` con soglie rigide attive.

- [ ] **Step 1:** implementare il banco e l'intestazione `Server-Timing`.
- [ ] **Step 2:** eseguire su `bench/data/big-100k` rigenerata (Task 22) sul PC di sviluppo. Atteso: ricerca p95 < 50 ms, primo byte p95 < 200 ms, zero file riletti nella seconda scansione; tempi della prima scansione e RAM annotati.
- [ ] **Step 3:** misurare l'indice con e senza i campi CJK (dimensione su disco e p95) e annotarlo (§11 punto 12).
- [ ] **Step 4:** scrivere `docs/benchmarks.md` (in inglese): comandi, macchina, risultati, limiti (SSD, dati sintetici).
- [ ] **Step 5: Commit** `bench: harness for the section 3 thresholds and CI smoke run`

### Task 24: Prova di un file HE-AAC (§11 punto 1)

**Files:**
- Create: `docs/spikes/07-he-aac.md`

- [ ] **Step 1:** procurarsi un file M4A HE-AAC di musica con licenza libera (il `ffmpeg` della CI non ha `libfdk_aac`; strade: un build di ffmpeg con `libfdk_aac`, oppure un file HE-AAC pubblicato con licenza Creative Commons; mai committarlo). Se non si trova in un'ora, scriverlo nel documento e rinviare alla fase 2.
- [ ] **Step 2:** verificare tre cose con un piccolo programma in `spikes/07-he-aac/` (escluso dal workspace): `probe::read_file` legge tag e durata; `content_hash` lo calcola; Symphonia decodifica il livello AAC-LC di base (con quale frequenza e durata).
- [ ] **Step 3:** scrivere l'esito in `docs/spikes/07-he-aac.md` e una riga nel design §7.2 con la data.
- [ ] **Step 4: Commit** `docs: he-aac probe results`

### Task 25: Uscita della fase 1 (con l'utente)

**Files:**
- Create: `docs/phase-1-acceptance.md`

- [ ] **Step 1: Installazione sul server di casa** con Docker (Task 21), libreria vera dell'utente (circa 500-1000 canzoni) montata in sola lettura, due utenti oltre all'admin (uno con solo ascolto e senza download offline).
- [ ] **Step 2: Symfonium e Feishin**, per ognuna: accesso con utente e password, sincronizzazione completa, navigazione per artista e album, ricerca con un errore di battitura, riproduzione con avanzamento nella traccia, copertine, preferito su traccia, album e artista, valutazione, creazione e modifica di una playlist, scrobbling su ListenBrainz, ripresa della coda su un secondo dispositivo, download offline negato all'utente senza permesso. Ogni voce: ok o problema, con versione dell'app.
- [ ] **Step 3: Spostamento a mano**: con un file manager spostare e rinominare un file con preferito e in una playlist; dopo la scansione automatica la traccia è la stessa nelle due app (stesso preferito, stessa posizione nella playlist). Ripetere spostando e ritaggando il file con un altro programma: la traccia torna dopo il passaggio in background.
- [ ] **Step 4: Disco staccato**: smontare il disco della libreria (o rinominare la cartella), attendere una scansione, rimontare: nessun dato personale perso.
- [ ] **Step 5: Misure su hardware vero** (decisione 7): generare con `gen-big` una libreria finta da 50.000 tracce sul disco di casa (HDD se c'è) ed eseguire `armonika-bench` sulla macchina di casa: prima scansione con 1, 4 e 8 thread (`scan.threads`), ricerca, primo byte, RAM. Se i 5 minuti per 50.000 file non sono raggiungibili, proporre all'utente la nuova soglia con il motivo (§11 punto 5). Sulla libreria vera: 30 ricerche scritte a mano (con errori, senza accenti, parti di titolo) e giudizio dell'utente sui primi risultati, annotato.
- [ ] **Step 6: Uso quotidiano** per almeno una settimana; problemi annotati in `docs/phase-1-acceptance.md`.
- [ ] **Step 7:** commit `docs: phase 1 acceptance results` e pull request M6 `Phase 1 M6: delivery and measurements`. La fase 1 è chiusa quando l'utente conferma il criterio di uscita del §9.

---

## Rinviato ad altre fasi (verificato contro §9)

- Impronta Chromaprint in background, edit list degli M4A, corpus Opus: fase 2 (decisione 3).
- Riconoscimento dell'atomo `keys` negli M4A, scrittura dei tag, livelli di scrittura oltre "Sola lettura", cestino, `inbox`, coda, limiti di spazio: fase 2 (la colonna `write_level` esiste già ma vale sempre `read_only`).
- Last.fm (decisione 4): dopo la fase 1, come seconda implementazione di `ScrobbleService`, con il collegamento dall'interfaccia della fase 2.
- Testi delle canzoni (`getLyrics`, estensione `songLyrics`), informazioni su artisti e album da servizi esterni: non previsti dal design per la fase 1.
- **Nota per la fase 2:** MusicBrainz unisce entità e reindirizza i vecchi MBID, quindi file taggati in anni diversi possono avere MBID diversi per lo stesso artista o la stessa edizione. Quando il riconoscimento lo scopre, servono l'unione di due artisti (con le regole dei dati personali del §8.7 punto 6 e `id_redirects`) e l'aggiornamento di `artists.mbid`, che oggi è `UNIQUE`.
- Binari su GitHub Releases: con il rilascio pubblico (fine fase 2).
- Esportazione dei dati personali (§8.1): fase 2, dall'interfaccia; in fase 1 basta la copia della cartella `data` (scritto in `docs/configuration.md`).
