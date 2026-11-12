# Spike 2: MeTube, API e recupero dei file

Data: 2026-11-12. Obiettivo (design §8.4): capire con una vera istanza MeTube come si invia una richiesta di download solo audio, come si legge lo stato, come si recupera il file finito via HTTP, e se il recupero via HTTP basta o serve la cartella condivisa. Il formato delle risposte di MeTube non è uno standard stabile, quindi tutto ciò che segue è stato osservato su un'istanza reale, non copiato dalla documentazione.

## 1. Versione provata

- Immagine: `ghcr.io/alexta69/metube:latest`, digest `ghcr.io/alexta69/metube@sha256:6654866e76eab4228530dc5ed828ccfd702d1ee810feab2d629028272b399e14`, creata il 2026-11-10.
- `GET /version` risponde `{"yt-dlp": "2026.10.26", "version": "2026.11.10"}`.
- Docker 28.4.0 su Windows 11 (Docker Desktop), cartella dei download montata da `spikes/02-metube/downloads/` (ignorata da git).
- Configurazione: solo `DELETE_FILE_ON_TRASHCAN=ask` (vedi §4.4), tutto il resto di default. `docker-compose.yml` in `spikes/02-metube/`.
- Fonti lette per capire i nomi dei campi: `app/main.py`, `app/ytdl.py`, `app/format_catalog.py` del ramo `master` e il bundle dell'interfaccia dentro l'immagine. Ogni comportamento citato sotto è stato poi riprodotto con il client.

## 2. Video usati

Dieci video musicali, tutti con la riga "Licenza" della pagina YouTube uguale a "Creative Commons Attribution license (reuse allowed)". Verifica: ho scaricato la pagina di ogni video (`https://www.youtube.com/watch?v=...`) e letto la voce "License" dei metadati della pagina; i candidati sono usciti dalla ricerca YouTube con il filtro "Creative Commons". Controllo fatto il 2026-11-12. La licenza è quella dichiarata da chi ha caricato il video: non ho verificato a monte i diritti del brano. Gli stessi dati sono in `spikes/02-metube/videos.txt`.

Tre video hanno un titolo con accenti (3, 6, 7), tre con caratteri non latini (giapponese 8 e 9, coreano 10).

| # | URL | Titolo | Canale | Licenza mostrata sulla pagina |
|---|---|---|---|---|
| 1 | https://www.youtube.com/watch?v=etR_OP3ay1s | Creative Commons Music  // epicmoment | Creative Commons Music | Creative Commons Attribution license (reuse allowed) |
| 2 | https://www.youtube.com/watch?v=5xYwDgpMe-I | Beautiful World/Free Music/ Creative Commons Music/ No Copyright Music / Motivational Music | Orchestralis / Background Music For Videos | Creative Commons Attribution license (reuse allowed) |
| 3 | https://www.youtube.com/watch?v=hhzFphMzgmw | [SIN COPYRIGHT] 🎵 Electrónica - Choices - PatrickReza 🎵 [CREATIVE COMMONS MUSIC] | Free Copyright Music 502 | Creative Commons Attribution license (reuse allowed) |
| 4 | https://www.youtube.com/watch?v=cIcgpKvh-r4 | Anthem of Inspiration / Piano Orchestral Background Music (Creative Commons) | Roman Senyk Music | Creative Commons Attribution license (reuse allowed) |
| 5 | https://www.youtube.com/watch?v=9h_bujcwPU0 | Beautiful Creative Commons Piano Music - "Aurora Sounds" | Echoes of Sadness | Creative Commons Attribution license (reuse allowed) |
| 6 | https://www.youtube.com/watch?v=UkjGJCpqSe0 | smoke de lish grooves musique libre de droit créative commons | libre tune | Creative Commons Attribution license (reuse allowed) |
| 7 | https://www.youtube.com/watch?v=-nBsRI_wwco | gran sentimiento de luna cantina musique libre de droit créative commons | libre tune | Creative Commons Attribution license (reuse allowed) |
| 8 | https://www.youtube.com/watch?v=pwSZ9Myrdjg | 【フリーBGM】Cobalt Planet | フィローネ | Creative Commons Attribution license (reuse allowed) |
| 9 | https://www.youtube.com/watch?v=kX8xc3TURy4 | かんがえごと - フリーBGM・インスト曲 / Daydreams - Free BGM, Instrumental | moja_manju | Creative Commons Attribution license (reuse allowed) |
| 10 | https://www.youtube.com/watch?v=YxpY5oP3Ukg | Creative commons 무료백그라운드음악  BGM - 아침에 눈을뜨면 네가 내 옆에 있어 (연주곡) 50번째 자작곡 | 노래창작Oppaya스튜디오 | Creative Commons Attribution license (reuse allowed) |

Nota: nel titolo del video 5 la pagina usa una lineetta lunga, qui sostituita da un trattino.

## 3. Client

`spikes/02-metube/` è un progetto Cargo autonomo (reqwest bloccante e serde_json, nessuna dipendenza dal workspace). Comandi:

- `run <urls.txt> <base_url> <downloads_dir> <fetch_dir>`: per ogni URL, aggiunge, segue lo stato fino a completamento o errore, scarica il file via HTTP, lo verifica con `ffprobe`, confronta la dimensione (e i byte) con il file sul disco, poi cancella l'elemento e il file da MeTube. Un rapporto JSON per riga.
- `add`, `watch`, `fetch`, `names`: strumenti per i casi di errore e per i nomi difficili.

Lo stato si segue interrogando `GET /history` ogni 500 ms (vedi §4.2). Non ho usato socket.io (vedi §4.5).

## 4. Endpoint e payload osservati

Base: `http://host:8081` (porta di default). Tutte le chiamate sotto sono senza credenziali.

### 4.1 Aggiungere un download

`POST /add`, `Content-Type: application/json`:

```json
{
  "url": "https://www.youtube.com/watch?v=etR_OP3ay1s",
  "download_type": "audio",
  "format": "auto",
  "quality": "best",
  "audio_tags": "with_cover"
}
```

- `download_type: "audio"` e `format: "auto"` scelgono il miglior flusso audio così come lo offre la sorgente, senza riconversione. `GET /formats` elenca le scelte (`auto`, `m4a`, `mp3`, `opus`, `wav`, `flac`); `m4a`, `mp3`, `wav`, `flac` convertono, quindi per "nessuna riconversione inutile" serve `auto`.
- `audio_tags: "with_cover"` (default) incorpora la copertina e scrive i tag.
- Risposta HTTP 200 in ogni caso, anche per gli errori. Successo: `{"status":"ok"}`. Errore immediato: `{"status":"error","msg":"..."}` (vedi §7). Un campo mancante o un valore non valido dà invece HTTP 400 con la ragione nella status line.
- Il vecchio formato della richiesta (`format` e `quality` senza `download_type`) è ancora accettato da una funzione di migrazione nel codice. Il formato è quindi cambiato nel tempo: il client di Armonika dovrà usare il formato nuovo e controllare `GET /version`.
- `/add` non restituisce un identificativo. L'elemento si ritrova in `/history` cercando per `url`.

### 4.2 Leggere lo stato

`GET /history` restituisce `{"done": [...], "queue": [...], "pending": [...]}`. Gli oggetti hanno gli stessi campi. Esempio (abbreviato, un video finito):

```json
{
  "id": "etR_OP3ay1s",
  "title": "Creative Commons Music  // epicmoment",
  "url": "https://www.youtube.com/watch?v=etR_OP3ay1s",
  "download_type": "audio", "format": "auto", "quality": "best", "folder": null,
  "status": "finished", "percent": 100.0, "size": 855788,
  "msg": null, "error": null,
  "filename": "Creative Commons Music  ⧸⧸ epicmoment.opus"
}
```

- Le liste sono indicizzate per `url`: è la chiave che MeTube usa per tutto (`/delete`, `/retry`). Aggiungere due volte lo stesso URL mentre il primo è ancora in `done` lascia il vecchio elemento `finished` in `done` mentre il nuovo scarica: il mio primo tentativo del secondo giro ha dato 404 perché il client ha scambiato il vecchio elemento per il nuovo e ha chiesto il file prima che esistesse. Armonika deve togliere da MeTube (o distinguere) l'elemento vecchio prima di aggiungere di nuovo lo stesso URL.
- Valori di `status` visti: `preparing`, `downloading`, `postprocessing` (in `queue`), `finished` e `error` (in `done`). Il codice ne prevede anche `pending` (aggiunto senza avvio automatico), `queued` e `scheduled`.
- Corrispondenza con gli stati mostrati da Armonika (design §8.4): in coda = `preparing` o `queued`; in corso = `downloading` o `postprocessing`; completato = `finished`; fallito = `error`. Qui "coda" è la coda di MeTube, cosa diversa dall'elemento in coda di Armonika (Glossario).
- `percent` va da 0 a 100 durante `downloading` ed è `null` durante `preparing` e a volte `postprocessing`; `size` si valorizza a fine download (per il file da 200 MB, già durante il `postprocessing` a 100%).
- La fase `postprocessing` (qui incorporamento della copertina, senza conversione) può durare: per il file da 200 MB oltre un minuto. Non va scambiata per un blocco.
- `filename` è il nome relativo alla cartella dei download (con `folder` se valorizzato). È presente solo da `finished`.
- Il titolo del file è quello del video con i caratteri vietati sostituiti (vedi §6).
- Lo stato è conservato da MeTube in `.metube/` dentro la cartella dei download e sopravvive al riavvio del contenitore (vedi §7.3).

### 4.3 Scaricare il file

`GET /audio_download/<percorso codificato>` per gli elementi audio (`GET /download/...` serve la stessa cartella con la stessa regola; il default `PUBLIC_HOST_AUDIO_URL` è `audio_download/`). Il percorso è `folder` (se non nullo) più `filename`, ogni componente codificato come `encodeURIComponent` di JavaScript, come fa l'interfaccia (`buildDownloadLink`). Il client `names` e `run` usano esattamente questa regola.

- Risposta: `200`, `Content-Type: audio/opus`, `Content-Length`, `Accept-Ranges: bytes` (le richieste `Range` danno `206`), `ETag`, `Last-Modified`. Server: aiohttp, file statico servito dalla cartella dei download.
- Senza credenziali e senza intestazioni speciali.

### 4.4 Altri endpoint usati

- `POST /delete` con `{"ids": ["<url>"], "where": "done", "delete_files": true}`: toglie l'elemento da `done`. Gli `ids` sono gli URL, non il campo `id` del video (provando con il campo `id` l'elemento è rimasto in `done`). `delete_files: true` cancella anche il file solo se `DELETE_FILE_ON_TRASHCAN` è `ask` o `true` (default `false`: il file resta). `where: "queue"` annulla un download in corso.
- `POST /retry` con `{"id": "<url>"}` rimette in coda un elemento fallito (letto dal codice, non provato).
- `GET /version`, `GET /formats` come sopra.

### 4.5 socket.io

L'interfaccia web non usa `/history` ma socket.io su `/socket.io`: eventi `all` (stato completo alla connessione), `added`, `updated` (avanzamento), `completed`, `canceled`, `cleared`. Non l'ho provato: il polling di `/history` basta (un secondo di ritardo è irrilevante per un download di musica) ed è più semplice da isolare e da testare. Un client di Armonika può usare solo HTTP.

## 5. Autenticazione

MeTube non ha autenticazione propria. Non esiste in `app/main.py` nessun controllo di utente, password, token o chiave: nessuna variabile d'ambiente per questo (la tabella delle variabili del README non ne ha), e tutte le chiamate di §4 funzionano senza credenziali, comprese `/history` e il download dei file. L'unica protezione nel codice è contro i siti web: un controllo dell'origine sulle richieste dei browser (`CORS_ALLOWED_ORIGINS`) e il blocco degli URL su indirizzi privati (SSRF, `ALLOW_PRIVATE_ADDRESSES`); le richieste senza `Origin` (curl, un client come Armonika) passano.

L'accesso si protegge davanti a MeTube con un reverse proxy (il README rimanda alla pagina "Reverse proxy configurations" della wiki, che parla di autenticazione). Conseguenze per Armonika (§8.4 prevede già "eventuali credenziali"): le credenziali sono quelle del proxy, quindi probabilmente HTTP Basic (o un'intestazione/token) da inviare a ogni richiesta, anche al download del file. Il client deve poter aggiungere intestazioni arbitrarie e seguire reindirizzamenti; non ho provato nessun proxy.

## 6. Formato e codec dei file

Tutti i 20 file (10 video, 2 giri) e il file lungo di §7.3 sono **Opus in contenitore Ogg**, estensione `.opus`. Nessun M4A: con `format: "auto"` yt-dlp sceglie come migliore il flusso Opus, che viene salvato così com'è (il file è un remux, non una riconversione). Non ho elencato i flussi offerti da YouTube, quindi non so se esistesse un M4A alternativo per ogni video.

| Quanti | Contenitore / codec | Copertina | Tag |
|---|---|---|---|
| 20 su 20 | `ogg` / `opus`, 48 kHz, 2 canali, 129,9-166,9 kbps (stima di `ffprobe`) | flusso `mjpeg` con `attached_pic=1` (la miniatura di YouTube; 1280x720 nei due file controllati) | nei tag del flusso audio (letti sui 10 file del primo giro): `title`, `artist`, `genre`, `date`, `purl`, `comment`, `synopsis`, `language`, `encoder` |

Dettagli utili per le fasi successive:

- Copertina incorporata: sì, in tutti i file, ma è la miniatura del video (16:9), non una copertina di album.
- `artist` è il nome del canale, `genre` è sempre `Music`, `title` è il titolo del video intero (con eventuali "[No Copyright]" ecc.). Non c'è `album`, numero di traccia né identificativi MusicBrainz. Servirà quindi il riconoscimento con AcoustID/MusicBrainz della coda, esattamente come per gli altri elementi in coda.
- I tag stanno nei metadati del flusso (non del formato): `ffprobe -show_format` non li mostra, serve `-show_streams`.
- Durata coerente con quella del video (per esempio 52.06 s per un video da 52 s).
- Il nome del file è il titolo del video con i caratteri non ammessi sostituiti da equivalenti a larghezza piena: `/` diventa `⧸`, `|` diventa `｜`, `"` diventa `＂`, `?` diventa `？`; gli spazi doppi restano. Lettere accentate, CJK, hangul ed emoji restano come sono (esempio: `[SIN COPYRIGHT] 🎵 Electrónica - Choices - PatrickReza 🎵 [CREATIVE COMMONS MUSIC].opus`). Il nome non è quindi riconducibile al titolo originale: Armonika deve leggere i tag, non il nome.
- Il nome è sicuro per il filesystem ma non necessariamente breve: mai verificato oltre 255 byte.
- Un file di 10.712 s (3 ore) pesa 201 MB.

## 7. Esiti

### 7.1 I 10 download, due volte (20 download)

Comando: `spike-metube run videos.txt http://localhost:8081 downloads fetch/run1` (e `run2`), con `DELETE_FILE_ON_TRASHCAN=ask` tra un giro e l'altro (il client cancella l'elemento e il file a ogni video).

| Misura | Giro 1 | Giro 2 |
|---|---|---|
| Elementi `finished` | 10 su 10 | 10 su 10 |
| Elementi `error` | 0 | 0 |
| `GET` del file: HTTP 200 | 10 su 10 | 10 su 10 |
| Dimensione scaricata uguale alla dimensione sul disco (e a `size` di MeTube) | 10 su 10 | 10 su 10 |
| Contenuto byte per byte uguale al file sul disco | 10 su 10 | 10 su 10 |
| `ffprobe` legge il file senza errori | 10 su 10 | 10 su 10 |
| Tempo da `/add` a `finished`, per video | 8,2-14,3 s | 7,9-13,6 s |

Totale: 20 su 20 download riusciti, 20 su 20 recuperi via HTTP corretti (56,1 MB scaricati in tutto). Dimensioni da 0,8 a 4,5 MB. Il secondo giro ha prodotto file della stessa dimensione e durata del primo.

Un incidente di metodo, già descritto in §4.2: il primo tentativo del secondo giro era sbagliato (elementi vecchi ancora in `done`, 404 sul file), e l'ho ripetuto tutto dopo aver corretto il client. I dati sopra sono quelli del giro corretto.

### 7.2 Nomi difficili

Il piano chiedeva di provare nomi con accenti, CJK, spazi e caratteri come `? # % &`. Con YouTube MeTube non produce `?` né `#`: li sostituisce già (§6), e NTFS (la cartella montata da Windows) non ammette `?`. Per provare davvero questi caratteri ho avviato una seconda istanza MeTube (stessa immagine, porta 8082) con un volume Docker al posto della cartella di Windows, ho creato dentro il contenitore 12 file Opus da 1 secondo con questi nomi e li ho scaricati con `names`.

| Nome del file | Percorso inviato | HTTP | Dimensione | ffprobe |
|---|---|---|---|---|
| `50%25 done%.opus` (% letterale, e `%25` letterale) | `50%2525%20done%25.opus` | 200 | uguale | ok |
| `Caf'e del Mar 東京 李白 한국어.opus` (CJK, hangul, apostrofo) | `Caf%27e%20del%20Mar%20%E6%9D%B1...` | 200 | uguale | ok |
| `Café NFC.opus` (é composta) | `Caf%C3%A9%20NFC.opus` | 200 | uguale | ok |
| `Café NFD.opus` (e + accento combinante) | `Cafe%CC%81%20NFD.opus` | 200 | uguale | ok |
| `a b  c.opus` (spazi doppi) | `a%20b%20%20c.opus` | 200 | uguale | ok |
| `back\slash.opus` | `back%5Cslash.opus` | 200 | uguale | ok |
| `emoji 🎵 test.opus` | `emoji%20%F0%9F%8E%B5%20test.opus` | 200 | uguale | ok |
| `quote"d and 'single'.opus` | `quote%22d%20and%20%27single%27.opus` | 200 | uguale | ok |
| `tilde~ at@ dollar$ comma, semi; colon: pipe\|.opus` | `tilde~%20at%40%20dollar%24%20comma%2C...` | 200 | uguale | ok |
| `what? #1 100% R&D + more=1;x.opus` | `what%3F%20%231%20100%25%20R%26D%20%2B%20more%3D1%3Bx.opus` | 200 | uguale | ok |
| `Ünïcödé Ñandú ß Æ.opus` | `%C3%9Cn%C3%AFc%C3%B6d%C3%A9%20%C3%91and%C3%BA%20%C3%9F%20%C3%86.opus` | 200 | uguale | ok |
| `..dots.. and (parens) [brackets] {braces}.opus` | `..dots..%20and%20(parens)%20%5Bbrackets%5D%20%7Bbraces%7D.opus` | 200 | uguale | ok |

12 su 12. Regole confermate:

- Si codifica ogni byte UTF-8 tranne `A-Z a-z 0-9 - _ . ! ~ * ( )` (equivalente a `encodeURIComponent`). NFC e NFD sono nomi diversi e vanno inviati come sono, senza normalizzare.
- `?` e `#` vanno sempre codificati (iniziano la query e il frammento di un URL); non ho provato a inviarli senza codifica. Un `+` non è uno spazio: `a+b++c.opus` dà 404, lo spazio va inviato come `%20`.
- `../../etc/passwd` dà 404 (nessuna uscita dalla cartella).
- Se `folder` è valorizzato, ogni componente del percorso va codificato separatamente e unito con `/`.

I file dei 20 download (nomi veri prodotti da YouTube, con accenti, giapponese, coreano ed emoji) sono stati scaricati con lo stesso metodo senza errori (§7.1).

### 7.3 Casi di errore

Tutti con `spike-metube add` e osservati in `/history`.

| Caso | Risposta di `/add` | Stato in `/history` | Messaggio (`msg`) |
|---|---|---|---|
| Testo che non è un link (`not a link`) | HTTP 200 `{"status":"error","msg":"ERROR: [generic] 'not a link' is not a valid URL"}` (immediata) | subito in `done`, `status: error`, `error` e `msg` valorizzati | `ERROR: [generic] 'not a link' is not a valid URL` |
| Host che non esiste (`https://example.invalid/video.mp4`) | HTTP 200 `{"status":"error","msg":"Could not resolve host \"example.invalid\""}` (immediata) | subito in `done`, `status: error`, `error` e `msg` valorizzati | `Could not resolve host "example.invalid"` |
| Video YouTube che non esiste/non disponibile (`...watch?v=zzzzzzzzzzz` e `aaaaaaaaaaa`) | HTTP 200 `{"status":"ok"}` (l'errore arriva dopo) | prima in `queue` (`postprocessing`, titolo provvisorio `youtube video #zzzzzzzzzzz`), poi in `done` con `status: error` entro pochi secondi; **`error` è `null`, il testo è solo in `msg`** | `[youtube] This video is unavailable\nNo video formats found!\nRequested format is not available\nERROR: [youtube] zzzzzzzzzzz: No video formats found!` |

Osservazioni:

- Per il caso "video rimosso" non ho usato un video veramente rimosso (non ne conosco uno stabile): un identificativo valido nella forma ma inesistente produce lo stesso messaggio "This video is unavailable" che yt-dlp dà per i video rimossi o resi privati. Non distinguibile da altri casi (privato, geobloccato) senza leggere il testo.
- Il fallimento può arrivare in due modi (risposta di `/add` con `status: error`, oppure più tardi in `/history`) e il campo che contiene il testo cambia (`error` o `msg`). Un client deve gestire entrambi e leggere `msg` quando `error` è `null`.
- Un errore resta in `done` finché non lo si toglie con `/delete` (o lo si rimette in coda con `/retry`, letto dal codice).
- Un elemento con `status: error` non ha `filename`: nessun file da recuperare.

### 7.4 MeTube fermato durante un download

Ho usato un video da 3 ore (https://www.youtube.com/watch?v=OOVMEuMyOHI, 10.713 s, licenza Creative Commons Attribution come gli altri) per avere il tempo di interromperlo. A circa il 16% (35 MB di `.webm.part` sul disco) ho eseguito `docker stop armonika-metube`, poi `docker start armonika-metube`.

- Dopo l'avvio, l'elemento era di nuovo presente nella `queue` di `/history`: stato ripristinato da `.metube/` senza intervento, senza richiedere un nuovo `/add`.
- Il download è **ripreso da solo e senza ripartire da zero**: il file parziale `.part` è stato riusato, il `.part` non è stato azzerato (35 MB allo stop, 123 MB un minuto dopo) e il primo `percent` letto dopo il riavvio era 21,4% (15,8% l'ultimo letto prima dello stop), fino al 100%.
- A fine corsa `status: finished`, `size` 201.040.686 byte, `GET` del file: 200, 201.040.686 byte, uguale al file sul disco byte per byte, `ffprobe` ok (Opus, durata 10.712,54 s, copertina presente).
- Mentre il contenitore è fermo MeTube è irraggiungibile (non l'ho cronometrato): il client deve tollerare errori di rete, riprovare e non segnare l'elemento come fallito.
- Non l'ho provato con lo stop durante `postprocessing`, né con `docker kill`: uno stop pulito sì, un'interruzione brusca (alimentazione) non è stata provata.

## 8. Conclusioni per Armonika

1. Il flusso aggiungi, controlla, scarica funziona solo con HTTP, senza cartella condivisa: 20 su 20, più il file lungo e 12 nomi difficili.
2. Il client va tenuto isolato (come previsto dal design §8.4): `/add` risponde in due modi, i campi `error` e `msg` si alternano, gli `ids` di `/delete` sono URL, lo stesso URL non si può riaggiungere senza prima togliere il vecchio elemento, il formato della richiesta è già cambiato una volta. `GET /version` va letto e registrato.
3. Il file ottenuto è Opus/Ogg con copertina miniatura e tag minimi (titolo del video, canale come `artist`): per Armonika è un elemento in coda che richiede il riconoscimento, non una traccia già a posto.
4. Senza `DELETE_FILE_ON_TRASHCAN` MeTube non cancella mai i file e Armonika non può pulire la cartella di MeTube: dopo il recupero i file restano nella cartella di MeTube a meno che l'utente imposti `ask` o `true`. È una scelta di installazione di MeTube, non qualcosa che Armonika possa imporre.
5. L'autenticazione dipende dal reverse proxy che l'utente mette davanti a MeTube: non provata qui (nessun proxy in questo spike).

## 9. Uscita

**Recupero via HTTP affidabile: sì.** Sull'istanza provata (MeTube 2026.11.10, yt-dlp 2026.10.26): 20 su 20 download e recuperi riusciti e verificati (dimensione, byte e `ffprobe`), 12 su 12 nomi difficili, e ripresa automatica dopo l'arresto del contenitore. La cartella condivisa resta un'opzione, non un obbligo.

Cosa serve, senza essere bloccante: (a) un client isolato che gestisca le anomalie di §8 punto 2; (b) la codifica del percorso come in §7.2.

Domanda per l'utente: la tua istanza MeTube è protetta da un reverse proxy con password (tipicamente HTTP Basic) o aperta in rete locale? Non so quale metodo di autenticazione dovrò supportare nelle impostazioni (§8.4) e non ho potuto provare nessun proxy; se mi indichi quale usi (nome del proxy e tipo di login), lo provo nella fase che implementa l'integrazione.
