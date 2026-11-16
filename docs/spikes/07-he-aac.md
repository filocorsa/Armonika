# Spike 7: prova di un file HE-AAC

Data: 2026-11-16. Obiettivo (design par. 7.2 e par. 11 punto 1): provare un M4A HE-AAC di musica. Tre domande: `probe::read_file` legge tag e durata? `content_hash` funziona? Symphonia decodifica il livello AAC-LC di base, e a quale frequenza e durata?

**Esito: tag, durata e impronta del contenuto funzionano; Symphonia 0.6.1 rifiuta il file ("aac too complex") e non decodifica nemmeno il livello di base.** Per l'impronta audio di un HE-AAC serve quindi il ripiego su ffmpeg già deciso (par. 11 punto 1), che lo decodifica correttamente.

## 1. Il file

- Origine: audio di YouTube nel formato 139 (`yt-dlp -f 139`), che YouTube codifica in HE-AAC. Il brano è protetto da diritto d'autore, non ha una licenza libera: è usato solo in locale per la prova e **non è committato** (sta in `bench/data/he-aac/`, ignorata da git). Il piano chiedeva un file con licenza libera; i tentativi precedenti di trovarne uno non sono riusciti (codificatore `aac_mf` di Windows solo LC, `libfdk_aac` assente, nessun HE-AAC con licenza Creative Commons trovato).
- `ffprobe`: AAC, profilo **HE-AAC**, 44100 Hz, 2 canali, 48 kbit/s, 213,16 s.
- Il file di YouTube non ha tag. Per provare la lettura dei tag è stata fatta una copia con tag aggiunti senza ricodifica: `ffmpeg -i yt139.m4a -c copy -metadata title=... -metadata artist=... -metadata album=... -metadata date=1987 -metadata track=1/10 tagged.m4a` (profilo ancora HE-AAC).

## 2. Comandi

Programma in `spikes/07-he-aac/` (fuori dal workspace), dalla sua cartella:

```
cargo run --release -- ../../bench/data/he-aac/yt139.m4a
cargo run --release -- ../../bench/data/he-aac/tagged.m4a
```

Macchina: AMD Ryzen 5 5600H, Windows 11, Symphonia 0.6.1 (`features = ["all"]`), lofty come in `armonika-media`.

## 3. Risultati

| Domanda | Esito |
|---|---|
| `probe::read_file`: tag | **Sì.** Titolo, artista, album, numero di traccia e data letti correttamente da `tagged.m4a`. |
| `probe::read_file`: durata | **Sì.** 213.159 ms, uguale a `ffprobe`. Bitrate 47 kbit/s, 2 canali. |
| `probe::read_file`: frequenza | **Imprecisa.** Riporta 22050 Hz, cioè la frequenza del livello AAC-LC di base, non i 44100 Hz di uscita. |
| `content_hash` | **Sì.** Calcolato (legge i pacchetti, senza decodifica). Identico tra `yt139.m4a` e `tagged.m4a`: cambiare i tag non cambia l'impronta del contenuto, come previsto. |
| Symphonia, decodifica | **No.** La creazione del decodificatore fallisce: `unsupported feature: aac: aac too complex`. Nessun campione decodificato, nemmeno del livello di base. Il contenitore dichiara 44100 Hz e 2 canali. |
| ffmpeg, decodifica (confronto) | **Sì.** Decodifica tutti i 213 s a 44100 Hz, 2 canali. |

## 4. Conseguenze

- **Fase 1: nessun problema.** Scansione, tag, durata, impronta del contenuto (riconoscimento dei file spostati) e ascolto senza transcodifica funzionano; la transcodifica usa già ffmpeg.
- **Impronta audio (fase 2, AcoustID):** l'ipotesi del design (Symphonia decodifica almeno il livello di base, quindi l'impronta potrebbe funzionare comunque) è **smentita**. Per un HE-AAC l'impronta si ottiene solo con ffmpeg; senza ffmpeg il file risulta "impronta audio non disponibile" e si riconosce dai tag. La decisione del par. 11 punto 1 resta valida così com'è.
- **Frequenza di campionamento riportata:** per gli HE-AAC Armonika mostra 22050 Hz invece di 44100 Hz (per esempio nel campo `samplingRate` dell'API Subsonic). Difetto minore, da valutare nella revisione di M6.

## 5. Limiti

- Un solo file, di una sola origine (YouTube, HE-AAC v1 a 48 kbit/s). HE-AAC v2 (con Parametric Stereo) non è provato.
- File non a licenza libera, usato solo in locale.
- Il messaggio "aac too complex" non è stato ricondotto alla riga del codice di Symphonia: si registra il comportamento osservato, non la causa interna.
