# Spike 6: decodifica dell'audio e impronta Chromaprint

Data: 2026-11-12. Obiettivo (design §7.2 e §11, spike 6): scegliere come decodificare l'audio di ogni formato accettato (MP3, FLAC, Ogg Vorbis, Opus, M4A/AAC, WAV, AIFF) e come calcolare l'impronta audio Chromaprint in modo che AcoustID la riconosca come quella di `fpcalc`. Tutti i numeri qui sotto vengono da esecuzioni reali del programma in `spikes/06-decode-fingerprint/` e da due esecuzioni della CI di GitHub, non dalla documentazione.

## Decisione

**Symphonia 0.6.1 per tutti i formati tranne Opus, `opus-pure` 0.2.3 per Opus, `rusty-chromaprint` 0.3.0 con `Configuration::preset_test2()` per l'impronta.** Nessuna dipendenza C, nessun processo esterno.

Regola del piano, applicata alla lettera:

| Criterio | Risultato della combinazione scelta |
|---|---|
| Decodifica il 100% dei formati accettati | Sì sui 57 file provati (MP3, FLAC, Ogg Vorbis, Opus, M4A AAC-LC, M4A ALAC, WAV, AIFF): 57 su 57. **HE-AAC non provato** (nessun file disponibile, vedi §6): è una domanda aperta per l'utente. |
| Stesso primo risultato AcoustID di `fpcalc` in almeno 19 casi su 20 | Sì: **20 su 20**, e in tutti e 20 la registrazione attesa (da `ground_truth.json`) è nel primo risultato. |
| Compila sui tre sistemi | Sì: la variante senza libopus compila su ubuntu-latest, windows-latest, macos-latest (2 esecuzioni su 2). |
| A parità, senza dipendenze C | Sì: è l'unica combinazione completa senza C e senza processi esterni. |

Anche le altre due varianti Opus complete (libopus tramite adapter, ffmpeg esterno) passano la regola sui 20 file, quindi la parità c'è e decide la preferenza per il Rust puro. In più la variante libopus ha perso audio su un file (§4.3) e `opus-decoder` è circa 140 volte più lento di `opus-pure` (§5), quindi non sono solo equivalenti ma peggiori.

Cose da fare nella fase di implementazione (non bloccano la scelta):

1. **M4A AAC: applicare a mano l'edit list (`elst`).** Symphonia 0.6.1 la ignora, quindi i 1024 campioni di priming dell'encoder restano in testa. L'impronta scende dal 99,4% medio al 97,2% medio di bit uguali con `fpcalc`; AcoustID dà comunque la stessa registrazione (4 su 4) ma con punteggio più basso (fino a 0,963 contro 0,984). Togliendo i 1024 campioni la somiglianza torna identica a quella di ffmpeg (§4.2). Va letto il `media_time` dell'edit list, non scritto 1024 fisso: con ALAC togliere 1024 campioni peggiora (99,02% scende a 96,89%).
2. **HE-AAC:** Symphonia dichiara solo AAC-LC. Serve una decisione dell'utente (§6).
3. Seguire le versioni giovani: `opus-pure` (prima 0.2.0 il 2026-08-31) e `rusty-chromaprint` 0.3.0 (ottobre 2024, nessuna versione dopo).

## 1. Versioni e ambiente

- Crate (da `spikes/06-decode-fingerprint/Cargo.lock`): `symphonia` 0.6.1 con feature `all` (MPL-2.0; `symphonia-codec-aac`, `symphonia-format-isomp4`, `symphonia-format-ogg` 0.6.1), `rusty-chromaprint` 0.3.0 (MIT; usa `rubato` 0.16.2 e `rustfft` 6.4.1), `opus-pure` 0.2.3 (BSD-3-Clause, nessuna dipendenza), `opus-decoder` 0.1.1 (MIT OR Apache-2.0), `symphonia-adapter-libopus` 0.3.0 (MIT OR Apache-2.0) con `opusic-sys` 0.7.7 (compila e include libopus, serve CMake), `base64` 0.22.1.
- Non provato: `symphonia-opus` 0.1.1, un decoder Opus in Rust puro per Symphonia pubblicato il giorno stesso dello spike (32 download): troppo nuovo per valutarlo.
- Riferimento: `fpcalc version 1.6.2 (FFmpeg Lavc62.11.100 Lavf62.3.100 SwR6.1.100)`. ffmpeg e ffprobe 6.1.1 per la variante ffmpeg. Rust 1.99.1.
- Macchina per i tempi: AMD Ryzen 5 5600H, 28 GB di RAM, Windows 11, SSD, un solo thread, build `--release`.
- Dati: i 54 file di `bench/data/messy/` (34 copie delle sorgenti in FLAC, MP3, Ogg Vorbis; 15 transcodifiche in altro formato, tra cui 6 Opus e 6 M4A; 5 copie da raccolta) più 3 file creati per lo spike in `bench/data/decode-extra/` da `nin-slip-02-copy.flac`: WAV (`pcm_s16le`), AIFF (`pcm_s16be`), M4A ALAC. Totale 57 file.
- Profili AAC dei 6 M4A (ffprobe): tutti `LC`. **Nessun HE-AAC** nella libreria disordinata.

## 2. Come è fatta la prova

Per ogni file (`spikes/06-decode-fingerprint/src/main.rs`):

1. Decodifica completa con ogni variante applicabile, in campioni `i16` interleaved:
   - `symphonia`: registro dei codec predefinito di Symphonia (tutti i formati; Opus fallisce perché non c'è il codec).
   - Solo per Opus: `symphonia+libopus` (Symphonia con `symphonia_adapter_libopus::OpusDecoder` registrato), `opus-pure` (lettore Ogg e decoder di `opus-pure`, con `Trim` per pre-skip ed end-trim), `opus-decoder` (`opus-decoder` non ha un demuxer Ogg: i pacchetti vengono dal lettore Ogg di `opus-pure` e il pre-skip si toglie a mano).
   - Solo per M4A: `symphonia+skip1024` (esperimento: Symphonia e poi si tolgono i primi 1024 campioni per canale, vedi §4.2).
   - `ffmpeg`: processo esterno `ffmpeg -i <file> -map 0:a:0 -f s16le -acodec pcm_s16le -`, frequenza e canali da ffprobe.
2. Impronta con `rusty-chromaprint`: `Configuration::preset_test2()` (l'algoritmo predefinito di `fpcalc`), `Fingerprinter::start(frequenza, canali)`, solo i primi 120 secondi di audio come fa `fpcalc` (`-length 120` predefinito). Il ricampionamento a 11025 Hz mono lo fa `rusty-chromaprint` al suo interno (`rubato`).
3. Riferimento: `fpcalc -raw -json <file>` (impronta grezza) e `fpcalc -json <file>` (impronta compressa).
4. Somiglianza: percentuale di bit uguali sulla parte sovrapposta, provando spostamenti da -20 a +20 elementi dell'impronta e tenendo il migliore. In tutti i 133 confronti il migliore è stato lo spostamento 0.
5. Compressione: `FingerprintCompressor` di `rusty-chromaprint` più base64 URL-safe senza padding. Verifica: comprimendo così l'impronta grezza di `fpcalc` si ottiene **esattamente** la stringa compressa di `fpcalc` in 57 file su 57. La compressione non introduce differenze.
6. AcoustID (`spikes/06-decode-fingerprint/acoustid_lookup.py`): 20 file della libreria disordinata, uno per sorgente, a quote per formato (6 Opus, 4 M4A, 4 FLAC, 3 MP3, 3 Ogg Vorbis, primi in ordine alfabetico). Lookup `POST https://api.acoustid.org/v2/lookup` con `meta=recordingids`, durata arrotondata al secondo, al massimo 3 richieste al secondo, User-Agent del progetto, risposte in cache in `bench/data/spike06/acoustid-cache/`. Un lookup per l'impronta di `fpcalc` e uno per l'impronta di `rusty-chromaprint` di ogni variante riuscita. "Primo risultato" è la prima registrazione del risultato con punteggio più alto.

## 3. Decodifica (Step 1)

File decodificati con successo, per formato e variante:

| Formato | File | symphonia | symphonia+libopus | opus-decoder | opus-pure | ffmpeg |
|---|---|---|---|---|---|---|
| FLAC | 13 | 13 | - | - | - | 13 |
| MP3 | 15 | 15 | - | - | - | 15 |
| Ogg Vorbis | 14 | 14 | - | - | - | 14 |
| Opus | 6 | **0** (codec non supportato) | 6 (1 con audio perso, §4.3) | 6 | 6 | 6 |
| M4A AAC-LC | 6 | 6 | - | - | - | 6 |
| M4A ALAC | 1 | 1 | - | - | - | 1 |
| WAV | 1 | 1 | - | - | - | 1 |
| AIFF | 1 | 1 | - | - | - | 1 |
| HE-AAC | 0 | non provato | - | - | - | non provato |

Errore di Symphonia sui file Opus, uguale per tutti e 6: `decoder: unsupported feature: core (codec): unsupported audio codec`. Symphonia non ha segnalato nessun `DecodeError` su nessun pacchetto; le altre varianti non hanno dato errori.

## 4. Impronte (Step 2)

### 4.1 Somiglianza con `fpcalc`

Percentuale di bit uguali tra l'impronta di `rusty-chromaprint` e quella di `fpcalc -raw` (spostamento migliore sempre 0, lunghezze sempre uguali):

| Formato | Variante | Min | Media | Max |
|---|---|---|---|---|
| FLAC (13) | symphonia | 98,253 | 99,281 | 99,759 |
| FLAC (13) | ffmpeg | 98,253 | 99,281 | 99,759 |
| MP3 (15) | symphonia | 98,243 | 99,240 | 99,934 |
| MP3 (15) | ffmpeg | 98,243 | 99,239 | 99,934 |
| Ogg Vorbis (14) | symphonia | 99,097 | 99,534 | 99,931 |
| Ogg Vorbis (14) | ffmpeg | 99,097 | 99,534 | 99,931 |
| Opus (6) | opus-pure | 99,044 | 99,498 | 99,832 |
| Opus (6) | opus-decoder | 99,041 | 99,497 | 99,832 |
| Opus (6) | symphonia+libopus | 95,675 | 98,862 | 99,832 |
| Opus (6) | ffmpeg | 99,044 | 99,497 | 99,832 |
| M4A AAC-LC (6) | symphonia | 96,776 | 97,158 | 97,946 |
| M4A AAC-LC (6) | symphonia+skip1024 | 98,836 | 99,367 | 99,835 |
| M4A AAC-LC (6) | ffmpeg | 98,836 | 99,367 | 99,838 |
| M4A ALAC (1) | symphonia | 99,024 | 99,024 | 99,024 |
| M4A ALAC (1) | symphonia+skip1024 | 96,888 | 96,888 | 96,888 |
| WAV (1) | symphonia | 99,024 | 99,024 | 99,024 |
| AIFF (1) | symphonia | 99,024 | 99,024 | 99,024 |

Nessuna impronta è identica bit per bit a quella di `fpcalc` (0 su 133), nemmeno partendo da WAV, dove Symphonia e ffmpeg danno lo stesso PCM. Siccome il risultato è uguale con ffmpeg come decoder, la differenza (circa 0,3-1,7% dei bit) non viene dalla decodifica ma dal calcolo, molto probabilmente dal ricampionamento a 11025 Hz (`rubato` in `rusty-chromaprint`, il ricampionatore di FFmpeg/Chromaprint in `fpcalc`). Non l'ho verificato oltre. Per AcoustID non conta (§5), ma vuol dire che il requisito "identica a quella di `fpcalc`" del design §7.2 è soddisfatto nel senso che conta (stesso riconoscimento), non bit per bit.

### 4.2 M4A AAC: edit list ignorata da Symphonia

I 6 M4A AAC-LC sono stati creati da ffmpeg, che scrive un'edit list con `media time: 1024` (ffprobe: `demuxer injecting skip 1024`). ffmpeg e `fpcalc` saltano quei 1024 campioni di priming; `symphonia-format-isomp4` 0.6.1 no (nel codice non c'è lettura di `elst`, il campo `delay` della traccia resta vuoto). Risultato: con Symphonia la somiglianza media è 97,158%; togliendo a mano i primi 1024 campioni è 99,367%, uguale a ffmpeg. Con ALAC (nessun priming) togliere 1024 campioni fa scendere da 99,024% a 96,888%: quindi va letto il valore dell'edit list, non usato un numero fisso.

### 4.3 Opus via libopus: audio perso su un file

Su `woodward-heretoday-01-opus.opus` la variante `symphonia+libopus` dà 95,675% contro 99,499% delle altre e un audio più corto di 0,142 s (13.270.608 campioni per canale contro 13.277.440 di `opus-pure` e ffmpeg). Il PCM coincide fino a circa 29,0 s e poi diverge. I lettori Ogg di Symphonia e di `opus-pure` vedono gli stessi 13.831 pacchetti e gli stessi 3.408.120 byte, ma la somma delle durate dei pacchetti calcolata da `symphonia-format-ogg` è 13.270.920 campioni, 6.520 in meno. L'ipotesi (non verificata nel codice dell'adapter) è che l'adapter tagli l'uscita alla durata del pacchetto data da Symphonia, e che Symphonia la calcoli male per qualche pacchetto. AcoustID dà comunque la registrazione giusta, con punteggio 0,926 invece di 0,965. Sugli altri 5 Opus la variante libopus coincide con le altre.

### 4.4 `opus-decoder`

Decodifica corretta (somiglianza praticamente uguale a `opus-pure`), ma l'uscita è più lunga di 0,013-0,019 s alla fine perché il mio codice non applica l'end-trim (lo fa solo `Trim` di `opus-pure`); non tocca l'impronta dei primi 120 s. Il problema è la velocità (§5).

## 5. AcoustID (Step 3)

Tra parentesi il punteggio AcoustID del primo risultato. "sì" vuol dire stessa prima registrazione di `fpcalc`.

| File | fpcalc: punteggio | fpcalc: registrazione attesa nel primo risultato | symphonia | symphonia+skip1024 | symphonia+libopus | opus-decoder | opus-pure | ffmpeg |
|---|---|---|---|---|---|---|---|---|
| bradsucks-mess-02-opus.opus | 0,987 | sì | - | - | sì (0,985) | sì (0,985) | sì (0,985) | sì (0,985) |
| nin-ghosts-01-opus.opus | 0,994 | sì | - | - | sì (0,993) | sì (0,993) | sì (0,993) | sì (0,993) |
| nin-ghosts-22-opus.opus | 0,990 | sì | - | - | sì (0,987) | sì (0,987) | sì (0,987) | sì (0,987) |
| nin-slip-01-opus.opus | 0,988 | sì | - | - | sì (0,988) | sì (0,988) | sì (0,988) | sì (0,988) |
| nin-slip-05-opus.opus | 0,989 | sì | - | - | sì (0,989) | sì (0,989) | sì (0,989) | sì (0,989) |
| woodward-heretoday-01-opus.opus | 0,966 | sì | - | - | sì (0,926) | sì (0,965) | sì (0,965) | sì (0,965) |
| bradsucks-mess-04-m4a.m4a | 0,984 | sì | sì (0,963) | sì (0,979) | - | - | - | sì (0,979) |
| nin-ghosts-02-m4a.m4a | 0,991 | sì | sì (0,990) | sì (0,991) | - | - | - | sì (0,991) |
| nin-ghosts-10-m4a.m4a | 0,968 | sì | sì (0,964) | sì (0,962) | - | - | - | sì (0,962) |
| nin-slip-02-m4a.m4a | 0,986 | sì | sì (0,966) | sì (0,982) | - | - | - | sì (0,982) |
| nin-ghosts-05-compilation.flac | 0,973 | sì | sì (0,973) | - | - | - | - | sì (0,973) |
| nin-ghosts-15-copy.flac | 1,000 | sì | sì (0,995) | - | - | - | - | sì (0,995) |
| nin-slip-03-compilation.flac | 1,000 | sì | sì (0,982) | - | - | - | - | sì (0,982) |
| nin-slip-04-copy.flac | 1,000 | sì | sì (0,996) | - | - | - | - | sì (0,996) |
| bradsucks-mess-01-copy.mp3 | 0,998 | sì | sì (0,989) | - | - | - | - | sì (0,989) |
| bradsucks-mess-03-copy.mp3 | 0,997 | sì | sì (0,986) | - | - | - | - | sì (0,986) |
| bradsucks-mess-05-compilation.mp3 | 0,998 | sì | sì (0,986) | - | - | - | - | sì (0,986) |
| nin-ghosts-28-copy.ogg | 0,994 | sì | sì (0,994) | - | - | - | - | sì (0,994) |
| nin-ghosts-36-copy.ogg | 0,986 | sì | sì (0,986) | - | - | - | - | sì (0,986) |
| nin-slip-09-copy.ogg | 0,983 | sì | sì (0,982) | - | - | - | - | sì (0,982) |

Riepilogo: per ogni variante riuscita, stessa prima registrazione di `fpcalc` in tutti i file in cui si applica; la registrazione di `ground_truth.json` è nel primo risultato in tutti i lookup (fpcalc e varianti). Combinazione scelta (Symphonia per i non Opus, `opus-pure` per Opus): **20 su 20**. Il punteggio di `rusty-chromaprint` è uguale o di poco più basso di quello di `fpcalc` (al massimo 0,018 in meno, a parte i casi spiegati in §4.2 e §4.3).

## 6. HE-AAC

Nessuno dei 6 M4A è HE-AAC (ffprobe: profilo `LC` per tutti). Ho provato a crearne uno, senza riuscirci:

- l'encoder `aac` di ffmpeg 6.1.1 non ha profili HE (solo LC, Main, LTP);
- `aac_mf` (Media Foundation) rifiuta `-profile:a aac_he` (`Unable to parse option value "aac_he"`);
- `libfdk_aac`, `fdkaac`, `qaac`, `neroAacEnc` non sono installati;
- scaricare un'immagine Docker di ffmpeg con `libfdk_aac` non mi è stato permesso in questa sessione.

Quindi su HE-AAC non c'è nessuna misura. Dalla documentazione Symphonia 0.6.1 supporta solo AAC-LC; ci sono `symphonia-adapter-fdk-aac` 0.2.0 (dipendenza C, FDK AAC) e ffmpeg esterno. **Domanda per l'utente:** gli M4A HE-AAC (tipici di alcuni download e di audio a basso bitrate) sono un formato da garantire? Se sì, le strade sono: ffmpeg come ripiego opzionale solo per i file che Symphonia non decodifica (nessuna dipendenza di compilazione), oppure l'adapter FDK (dipendenza C, da provare sulla CI come libopus). Prima di decidere servirebbe un file HE-AAC di prova creato con un encoder che lo supporti.

## 7. Compilazione multipiattaforma (Step 4)

Workflow temporaneo `.github/workflows/spike-libopus.yml` (rimosso nel commit `ci: remove temporary libopus spike workflow`): su ubuntu-latest, windows-latest, macos-latest compila `spikes/06-decode-fingerprint` in `--release --locked` prima con libopus (feature predefinita, `opusic-sys` compila libopus incluso con CMake), poi dopo `cargo clean` senza (`--no-default-features`: Symphonia, `opus-pure`, `opus-decoder`, `rusty-chromaprint`). Toolchain 1.99.1 (da `rust-toolchain.toml`), CMake 4.4.5 già presente sui runner.

| Esecuzione | Sistema | Esito | Build con libopus (s) | Build Rust puro (s) | Durata del job |
|---|---|---|---|---|---|
| 43296081173 (push) | ubuntu-latest | ok | 100 | 74 | 3 min 23 s |
| 43296081173 (push) | windows-latest | ok | 171 | 139 | 5 min 57 s |
| 43296081173 (push) | macos-latest | ok | 28 | 19 | 1 min 4 s |
| 43296108810 (`gh workflow run`) | ubuntu-latest | ok | 77 | 59 | 2 min 42 s |
| 43296108810 (`gh workflow run`) | windows-latest | ok | 126 | 100 | 4 min 32 s |
| 43296108810 (`gh workflow run`) | macos-latest | ok | 28 | 18 | 1 min 4 s |

La variante libopus compila sui tre sistemi senza interventi (CMake è già sui runner di GitHub; in locale su Windows c'era). Costa da 9 a 32 secondi in più di build da zero. Per un utente che compila da sorgente vorrebbe dire avere CMake e un compilatore C; per i binari distribuiti non cambia nulla. Non serve, perché la variante Rust pura basta.

## 8. Tempi (combinazione scelta)

Decodifica dell'intero file più impronta dei primi 120 s, un thread, 57 file (durata media 215,7 s):

| Misura | Mediana | Media | Min | Max |
|---|---|---|---|---|
| Decodifica completa (ms) | 185,5 | 207,6 | 53,1 | 730,1 |
| Impronta, 120 s (ms) | 40,3 | 41,8 | 28,4 | 85,9 |
| Totale (ms) | 227,5 | 249,5 | 81,5 | 773,4 |
| `fpcalc -raw`, processo completo, solo 120 s (ms) | 136,5 | 152,7 | 60,0 | 567,2 |

Mediana della sola decodifica per secondo di audio: AIFF 0,25 ms, WAV 0,31, FLAC 0,68, M4A 0,65 (6 AAC più 1 ALAC, che da solo è 730 ms), Ogg Vorbis 0,85, MP3 0,96, Opus (`opus-pure`) 2,01. Per Opus, sui 6 file: `symphonia+libopus` 1,83 ms per secondo di audio, `opus-pure` 2,01, ffmpeg esterno 3,15, `opus-decoder` **285** (mediana 57 s per file: inutilizzabile).

La mia decodifica è di tutto il file, mentre per l'impronta bastano i primi 120 s: decodificando solo quelli il costo scenderebbe (non misurato). Con la mediana misurata, 50.000 file costerebbero circa 3,2 ore di un thread: va bene per il calcolo in background dopo la scansione previsto dal design (§8.7), non durante la scansione (§3). Per lo spike 5: l'impronta del contenuto, secondo il design §8.7, si calcola senza decodificare, quindi questi tempi non la riguardano direttamente; se servisse decodificare, il costo è quello della riga "Decodifica completa".

Tutte le misure in `bench/data/spike06/results.json` (ignorato da git), file per file.

## 9. Tabella per file (combinazione scelta)

| File | Formato | Decoder | Durata (s) | Somiglianza con fpcalc (%) | Decodifica (ms) | Impronta (ms) | Totale (ms) | fpcalc (ms) |
|---|---|---|---|---|---|---|---|---|
| nin-slip-02-aiff.aiff | aiff | symphonia | 236,2 | 99,024 | 58 | 41 | 99 | 64 |
| nin-slip-02-alac.m4a | m4a | symphonia | 236,2 | 99,024 | 730 | 43 | 773 | 297 |
| nin-slip-02-wav.wav | wav | symphonia | 236,2 | 99,024 | 74 | 51 | 125 | 60 |
| bradsucks-mess-01-copy.mp3 | mp3 | symphonia | 201,0 | 98,988 | 437 | 86 | 523 | 146 |
| bradsucks-mess-02-copy.mp3 | mp3 | symphonia | 205,5 | 99,061 | 319 | 52 | 371 | 567 |
| bradsucks-mess-02-opus.opus | opus | opus-pure | 205,5 | 99,044 | 428 | 46 | 474 | 271 |
| bradsucks-mess-03-copy.mp3 | mp3 | symphonia | 211,5 | 98,708 | 204 | 40 | 244 | 139 |
| bradsucks-mess-04-copy.mp3 | mp3 | symphonia | 197,0 | 98,826 | 191 | 40 | 231 | 131 |
| bradsucks-mess-04-m4a.m4a | m4a | symphonia | 197,0 | 96,776 | 154 | 42 | 196 | 112 |
| bradsucks-mess-05-compilation.mp3 | mp3 | symphonia | 215,9 | 98,774 | 202 | 42 | 244 | 137 |
| bradsucks-mess-05-copy.mp3 | mp3 | symphonia | 215,9 | 98,774 | 201 | 42 | 243 | 131 |
| nin-ghosts-01-copy.flac | flac | symphonia | 168,8 | 99,759 | 111 | 53 | 164 | 128 |
| nin-ghosts-01-opus.opus | opus | opus-pure | 168,8 | 99,832 | 330 | 42 | 372 | 228 |
| nin-ghosts-02-copy.mp3 | mp3 | symphonia | 196,1 | 99,838 | 187 | 40 | 228 | 128 |
| nin-ghosts-02-m4a.m4a | m4a | symphonia | 196,2 | 97,946 | 122 | 39 | 161 | 96 |
| nin-ghosts-05-compilation.flac | flac | symphonia | 171,8 | 99,374 | 98 | 39 | 138 | 122 |
| nin-ghosts-05-copy.flac | flac | symphonia | 171,8 | 99,374 | 99 | 40 | 139 | 140 |
| nin-ghosts-07-copy.mp3 | mp3 | symphonia | 120,5 | 98,734 | 117 | 40 | 156 | 129 |
| nin-ghosts-10-copy.flac | flac | symphonia | 162,3 | 99,380 | 108 | 41 | 149 | 136 |
| nin-ghosts-10-m4a.m4a | m4a | symphonia | 162,4 | 97,168 | 105 | 51 | 156 | 93 |
| nin-ghosts-11-copy.mp3 | mp3 | symphonia | 137,2 | 99,647 | 131 | 40 | 172 | 129 |
| nin-ghosts-15-copy.flac | flac | symphonia | 113,2 | 99,724 | 77 | 42 | 119 | 126 |
| nin-ghosts-19-copy.mp3 | mp3 | symphonia | 131,7 | 98,243 | 128 | 41 | 169 | 130 |
| nin-ghosts-22-copy.flac | flac | symphonia | 151,1 | 99,354 | 90 | 41 | 131 | 125 |
| nin-ghosts-22-opus.opus | opus | opus-pure | 151,1 | 99,265 | 311 | 42 | 353 | 237 |
| nin-ghosts-25-copy.mp3 | mp3 | symphonia | 118,8 | 99,883 | 118 | 40 | 158 | 126 |
| nin-ghosts-28-copy.ogg | ogg | symphonia | 322,0 | 99,931 | 202 | 40 | 242 | 97 |
| nin-ghosts-36-copy.ogg | ogg | symphonia | 139,0 | 99,809 | 83 | 42 | 125 | 104 |
| nin-ghosts-36-mp3.mp3 | mp3 | symphonia | 139,0 | 99,789 | 145 | 39 | 184 | 120 |
| nin-slip-01-copy.flac | flac | symphonia | 85,2 | 99,737 | 53 | 28 | 81 | 103 |
| nin-slip-01-opus.opus | opus | opus-pure | 85,2 | 99,775 | 165 | 30 | 194 | 164 |
| nin-slip-02-copy.flac | flac | symphonia | 236,2 | 99,024 | 181 | 38 | 220 | 145 |
| nin-slip-02-m4a.m4a | m4a | symphonia | 236,2 | 96,964 | 142 | 40 | 182 | 90 |
| nin-slip-03-compilation.flac | flac | symphonia | 229,5 | 98,253 | 173 | 40 | 213 | 146 |
| nin-slip-03-copy.flac | flac | symphonia | 229,5 | 98,253 | 174 | 40 | 214 | 148 |
| nin-slip-04-copy.flac | flac | symphonia | 259,2 | 99,558 | 189 | 40 | 229 | 144 |
| nin-slip-04-ogg.ogg | ogg | symphonia | 259,2 | 99,595 | 159 | 40 | 199 | 103 |
| nin-slip-05-copy.flac | flac | symphonia | 285,2 | 99,532 | 195 | 39 | 235 | 139 |
| nin-slip-05-opus.opus | opus | opus-pure | 285,2 | 99,571 | 557 | 42 | 599 | 522 |
| nin-slip-06-copy.mp3 | mp3 | symphonia | 295,5 | 99,641 | 270 | 40 | 310 | 138 |
| nin-slip-07-copy.mp3 | mp3 | symphonia | 209,6 | 99,766 | 154 | 39 | 193 | 108 |
| nin-slip-08-copy.mp3 | mp3 | symphonia | 453,7 | 99,934 | 388 | 40 | 427 | 117 |
| nin-slip-09-copy.ogg | ogg | symphonia | 277,6 | 99,862 | 204 | 40 | 244 | 146 |
| nin-slip-09-m4a.m4a | m4a | symphonia | 277,7 | 96,944 | 186 | 43 | 229 | 103 |
| nin-slip-10-compilation.ogg | ogg | symphonia | 299,4 | 99,644 | 189 | 40 | 229 | 114 |
| nin-slip-10-copy.ogg | ogg | symphonia | 299,4 | 99,644 | 218 | 44 | 262 | 136 |
| woodward-heretoday-01-copy.ogg | ogg | symphonia | 276,6 | 99,522 | 264 | 41 | 305 | 160 |
| woodward-heretoday-01-opus.opus | opus | opus-pure | 276,6 | 99,499 | 572 | 41 | 614 | 236 |
| woodward-heretoday-02-copy.ogg | ogg | symphonia | 243,1 | 99,552 | 227 | 40 | 267 | 153 |
| woodward-heretoday-03-copy.ogg | ogg | symphonia | 286,5 | 99,506 | 239 | 39 | 279 | 138 |
| woodward-heretoday-04-copy.ogg | ogg | symphonia | 239,1 | 99,351 | 207 | 39 | 246 | 147 |
| woodward-heretoday-04-m4a.m4a | m4a | symphonia | 239,2 | 97,152 | 149 | 40 | 189 | 94 |
| woodward-heretoday-05-compilation.ogg | ogg | symphonia | 225,0 | 99,097 | 199 | 41 | 241 | 153 |
| woodward-heretoday-05-copy.ogg | ogg | symphonia | 225,0 | 99,097 | 204 | 39 | 243 | 151 |
| woodward-heretoday-06-copy.ogg | ogg | symphonia | 206,5 | 99,334 | 179 | 40 | 218 | 147 |
| woodward-heretoday-06-flac.flac | flac | symphonia | 206,5 | 99,337 | 173 | 40 | 212 | 158 |
| woodward-heretoday-07-copy.ogg | ogg | symphonia | 284,7 | 99,535 | 265 | 39 | 304 | 156 |

## 10. Come riprodurre

Da Git Bash, nella radice del repository, dopo aver generato la libreria disordinata (Task 5):

```bash
# File extra: WAV, AIFF, ALAC
mkdir -p bench/data/decode-extra
src=bench/data/messy/nin-slip-02-copy.flac
ffmpeg -v error -y -i $src -map 0:a -c:a pcm_s16le bench/data/decode-extra/nin-slip-02-wav.wav
ffmpeg -v error -y -i $src -map 0:a -c:a pcm_s16be bench/data/decode-extra/nin-slip-02-aiff.aiff
ffmpeg -v error -y -i $src -map 0:a -c:a alac bench/data/decode-extra/nin-slip-02-alac.m4a

# Profili AAC
for f in bench/data/messy/*.m4a; do ffprobe -v error -select_streams a:0 -show_entries stream=profile -of csv=p=0 "$f"; done

# Decodifica e impronte (circa 7 minuti, quasi tutti di opus-decoder)
cargo build --release --manifest-path spikes/06-decode-fingerprint/Cargo.toml
mkdir -p bench/data/spike06
spikes/06-decode-fingerprint/target/release/spike-decode-fingerprint \
  bench/data/spike06/results.json bench/data/messy bench/data/decode-extra

# Diagnosi del file Opus con libopus (pacchetti visti dai due lettori Ogg)
spikes/06-decode-fingerprint/target/release/spike-decode-fingerprint count-packets \
  bench/data/messy/woodward-heretoday-01-opus.opus

# AcoustID (la chiave non viene mai stampata)
export ARMONIKA_ACOUSTID_KEY="$(powershell -NoProfile -Command "[Environment]::GetEnvironmentVariable('ARMONIKA_ACOUSTID_KEY','User')" | tr -d '\r\n')"
python spikes/06-decode-fingerprint/acoustid_lookup.py \
  bench/data/spike06/results.json bench/data/messy/ground_truth.json bench/data/spike06/acoustid.json

# Build senza libopus (solo Rust)
cargo build --release --no-default-features --manifest-path spikes/06-decode-fingerprint/Cargo.toml
```

Con `SPIKE_DUMP_DIR=<cartella>` il programma scrive anche il PCM decodificato di ogni variante (s16le), usato per trovare il punto in cui l'uscita di libopus diverge.
