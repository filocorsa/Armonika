# Spike 4: riconoscimento e punteggio di confidenza

Data: 2026-11-12. Obiettivo (design §8.2 e §11, spike 4): misurare per quanti file AcoustID + MusicBrainz trovano la registrazione e l'edizione giuste, e stabilire con quali pesi e soglie il punteggio di confidenza di Armonika (da 0 a 100) separa le corrispondenze giuste da quelle sbagliate. Tutti i numeri qui sotto vengono da esecuzioni reali dei programmi in `spikes/04-recognition/` sulla libreria disordinata (`bench/data/messy`, 54 file), con le risposte di AcoustID e MusicBrainz salvate in cache.

> **Avvertenza.** Il campione è piccolo: 54 file, 34 registrazioni, 4 edizioni di 3 artisti, nessun file che venga davvero da una raccolta, da un singolo o da un live. Pesi e soglie qui sotto sono quelli che funzionano **su questo campione**; vanno ricontrollati in fase 2 su file veri (più artisti, raccolte vere, file con registrazioni sbagliate o assenti da MusicBrainz) prima di usarli per la conferma in blocco.

## Decisione

**Punteggio di un file** (per una coppia registrazione + edizione candidata):

```
punteggio = 100 * (1*A + 2*D + 1*T + 1*R + 1*O) / (somma dei pesi degli indizi presenti)
```

| Indizio | Peso | Valore (da 0 a 1) |
|---|---|---|
| A, AcoustID | 1 | `score` del risultato AcoustID che contiene la registrazione (risultati sotto 0,5 scartati) |
| D, durata | 2 | `max(0, 1 - |durata del file - durata della registrazione| / 10 s)`; durata della registrazione da MusicBrainz (`length`) |
| T, titolo | 1 | Jaro-Winkler tra il tag titolo **ripulito** e il titolo della registrazione, entrambi normalizzati (§3) |
| R, artista | 1 | Jaro-Winkler tra il tag artista e l'accredito della registrazione, normalizzati |
| O, edizione originale | 1 | 1 se l'edizione è ufficiale (`status = Official`) e il suo album è di tipo `Album` senza tipi secondari (niente `Compilation`, `Remix`, `Soundtrack`, `Live`...), altrimenti 0 |

- **Tag mancante**: T o R non entrano nel punteggio (né il valore né il peso). Un file senza tag è giudicato solo su A, D e O.
- **Edizione del gruppo**: per ogni edizione candidata, punteggio del gruppo = media sui file del gruppo del miglior punteggio del file su quell'edizione; un file che non ha nessuna registrazione candidata in quell'edizione conta 0. Si propone l'edizione con il punteggio del gruppo più alto; a parità, la data di pubblicazione più vecchia (date incomplete dopo quelle complete). I file del gruppo che non stanno nell'edizione scelta ricevono la loro proposta migliore da soli (Armonika propone di separarli dal gruppo, design §8.7 punto 15).
- **Somiglianza del titolo dell'album, frazione di file che corrispondono all'edizione, numero di tracce uguale**: peso 0. Su questo campione non migliorano nulla (§4.3); la frazione di file conta comunque nel punteggio del gruppo attraverso la media (un file fuori dall'edizione conta 0).

**Soglie** (stesse per file e gruppi):

| Soglia | Valore | Su questo campione |
|---|---|---|
| "Corrispondenza incerta" | punteggio **sotto 85** | 0 proposte giuste sotto 85; tutti i candidati sbagliati (§5) sono sotto 85 (il più alto 83,20) |
| "Conferma tutti quelli sopra la soglia" | punteggio del gruppo **almeno 90** | 0 gruppi sbagliati sopra 90; 0 gruppi giusti sotto 90 su 24 |

Risultati con questi pesi (54 file, 24 gruppi):

| Misura | Risultato |
|---|---|
| Registrazione giusta al primo posto | **54/54 (100%)** |
| Album giusto (release group) al primo posto, senza le copie da raccolta | **49/49 (100%)** |
| Edizione esatta giusta al primo posto, senza le copie da raccolta | **47/49 (95,9%)** (§6.1) |
| Copie da raccolta: registrazione giusta / edizione proposta = edizione originale della ground truth | 5/5 / 5/5 (4/5 come edizione esatta) |
| Proposte sbagliate sopra 85 (e sopra 90) | 0 |
| Candidati sbagliati (miglior candidato sbagliato di ogni file) sopra 85 | 0 su 35 file che ne hanno uno |
| Proposte giuste sotto 90 | 0 (la più bassa 93,66) |
| Gruppi giusti sotto 90 / gruppi sbagliati sopra 90 | 0/23 / 0/1 |

Margine tra il candidato sbagliato più alto (83,20) e la proposta giusta più bassa (93,66): **10,46 punti**. Le soglie 85 e 90 stanno in quel buco.

**Domande e avvisi per l'utente:**

1. **Con questa formula un'edizione che non è l'album originale (raccolta, singolo, EP, live, remix, bootleg) non può superare 83,3** (manca il peso di O, 1 su 6; 75 se il file non ha tag). Quindi un file che viene davvero da una raccolta finisce sempre in "corrispondenza incerta" e non viene mai confermato in blocco, anche se la proposta è giusta. È prudente, ma il design chiede di preferire l'originale "salvo indizi contrari" (es. tutti i file del gruppo corrispondono a una raccolta): su questo campione non c'è nessuna raccolta vera, quindi quegli indizi (titolo dell'album, frazione di file) non si possono tarare. Proposta, non misurata: in fase 2 mettere O = 1 anche quando l'edizione copre tutti i file di un gruppo di almeno 2 file e il titolo dell'album dei tag le somiglia (Jaro-Winkler almeno 0,9), e ripetere la misura con raccolte vere.
2. **Il peso della durata poggia su 2 file** (le due copie di `nin-ghosts-22`, dove AcoustID dà con lo stesso punteggio anche la registrazione di "26 Ghosts III"). Togliendo l'edizione Ghosts I-IV dalla taratura, la griglia sceglie peso 0 per la durata e quei 2 file hanno un candidato sbagliato a 98,95 (§4.4).
3. **Proposta di sicurezza, non misurata**: nessuna proposta con score AcoustID sotto 0,9 sopra la soglia di conferma in blocco. Nel campione la proposta giusta ha sempre AcoustID almeno 0,963, ma la formula è lineare e con A = 0,6 e tutto il resto perfetto darebbe 93,3: non ci sono casi così nel campione per dire se sono giusti o sbagliati.

## 1. Come è fatta la prova

1. `spikes/04-recognition/src/main.rs` (Rust, scelte dello spike 6): decodifica con Symphonia 0.6.1 (Opus con `opus-pure` 0.2.3), impronta Chromaprint dei primi 120 s con `rusty-chromaprint` 0.3.0 (`preset_test2`), compressa come `fpcalc`. Tag letti con `lofty` 0.25.5 (titolo, artista, album, artista dell'album, numero e totale delle tracce). Durata = campioni decodificati / frequenza. Nota: Symphonia ignora l'edit list degli M4A (1024 campioni di priming in più, spike 6 §4.2); qui non cambia nulla.
2. `spikes/04-recognition/fetch.py`: per ogni file lookup AcoustID (`POST /v2/lookup`, `meta=recordings releases releasegroups`, durata arrotondata al secondo), poi per ogni registrazione dei risultati con score almeno 0,5 `GET /ws/2/recording/<id>?inc=releases+release-groups+media+artist-credits` su MusicBrainz (titolo, accredito, durata, edizioni con stato, data, tipo dell'album e supporti). User-Agent `Armonika/0.0.0 ( https://github.com/filocorsa/Armonika )`, al massimo 3 richieste al secondo ad AcoustID e 1 ogni 1,1 s a MusicBrainz. Ogni risposta è salvata in `bench/data/recognition-cache/` (ignorata da git) con chiave = hash della richiesta senza la chiave AcoustID; la chiave non viene mai scritta né stampata. Prima esecuzione: 97 richieste di rete (49 AcoustID, 48 MusicBrainz) e 5 risposte dalla cache (le 5 copie da raccolta hanno lo stesso audio dell'originale, quindi la stessa richiesta). Seconda esecuzione: **0 richieste di rete**, 102 dalla cache.
3. `spikes/04-recognition/recognize.py` (solo cache, nessuna rete): indizi, gruppi, ricerca su griglia, soglie, controprova lasciando fuori un'edizione. Numero di tracce dell'edizione dal campo `track_count` di AcoustID (totale su tutti i supporti), altrimenti dal supporto MusicBrainz se è uno solo.
4. Gruppi come nel design §8.7 punto 15, prima del riconoscimento: stessa cartella (qui tutte) e stessi tag titolo dell'album e artista dell'album; un file senza titolo dell'album è un gruppo da solo. Risultano **24 gruppi**: 7 con più file (fino a 11), 17 da un file. Le 5 copie da raccolta (album "Greatest Hits", artista dell'album "Various Artists") formano un gruppo unico con file di 4 edizioni diverse.

Ground truth: `bench/data/messy/ground_truth.json` (registrazione ed edizione per file). Danni per file da `bench/messy/manifest.toml`: il file `-copy` ha i danni che cambiano i tag (`youtube_title`, `strip_tags`, `strip_cover`, `split_album`, `wrong_case`, anche più di uno), i file derivati sono `duplicate_other_format` o `compilation_copy` e partono dalla sorgente pulita. Un file `-copy` senza danni è "clean" (11 file).

Nota sui tag: le transcodifiche `nin-slip-09-m4a`, `woodward-heretoday-04-m4a`, `woodward-heretoday-06-flac` e `nin-ghosts-36-mp3` (dalle sorgenti Ogg) non hanno nessun tag: ffmpeg non ha copiato i commenti Vorbis. Nei fatti sono altri 4 file senza tag, oltre ai 5 `strip_tags`.

## 2. Candidati (Step 1)

- AcoustID trova la registrazione della ground truth fra i candidati in **54/54** file, sempre nel risultato con lo score più alto. Score AcoustID della registrazione giusta: da 0,963 a 0,999.
- 77 coppie file + registrazione candidate (48 registrazioni distinte, da 1 a 3 per file) e 423 coppie file + edizione candidate (in media 7,8 edizioni per file; i brani di Ghosts I-IV compaiono in 10-11 edizioni dello stesso album).
- In **18 file su 54** lo score più alto è condiviso da 2 o 3 registrazioni diverse, quindi AcoustID da solo non decide:
  - `nin-ghosts-22` (2 file): "22 Ghosts III" e "26 Ghosts III", stesso score; la durata (151 s contro 146 s) le distingue.
  - `nin-slip-08`, `nin-slip-09` (3 file): la registrazione e la sua versione "(instrumental)" di "The Slip (instrumental)", stesso score e durata quasi uguale. Su MusicBrainz quell'edizione è un bootleg, quindi O = 0.
  - `nin-slip-10` (2 file): "Demon Seed", "Demon Seed (instrumental)" e una seconda registrazione "Demon Seed" che MusicBrainz ha solo su un bootleg russo del 2008 intitolato proprio **"Greatest Hits"** (come l'album finto delle nostre copie da raccolta).
  - Josh Woodward (11 file): la registrazione del 2004 ("Here Today") e quella del 2008 ("Here Today (Remixed)", album con tipo secondario `Remix`), stessa impronta. La durata favorisce il 2008 in 5 brani su 7 (es. `woodward-heretoday-01`: file 276,6 s, 2004 278,28 s, 2008 276,0 s): decide O.

## 3. Indizi (Step 2)

### 3.1 Testo: normalizzazione e pulizia

- **Normalizzazione** (sempre, su entrambi i lati): Unicode NFKD, via gli accenti, minuscole, ogni carattere non alfanumerico diventa spazio, spazi compattati. Serve anche per MusicBrainz, che usa apostrofi e trattini tipografici (es. "Guess Who's a Mess" con U+2019, "Ghosts I-IV" con U+2013).
- **Pulizia** (solo sul tag titolo, mai sui titoli MusicBrainz, perché "(instrumental)" deve restare): via le parentesi tonde o quadre che contengono official, video, audio, lyric(s), hd, hq, 4k, visualizer, mv; via "feat./ft./featuring ..."; via il prefisso "Artista - " se coincide con il tag artista.

Jaro-Winkler tra tag titolo e titolo della registrazione giusta:

| Danno | File con titolo | Solo minuscole | Normalizzato, senza pulizia | Normalizzato e pulito |
|---|---|---|---|---|
| `youtube_title` | 6 | min 0,568, media 0,615 | min 0,579, media 0,616 | **1,000** in tutti |
| `wrong_case` | 5 | 1,000 | 1,000 | 1,000 |
| `split_album`, `compilation_copy`, clean | 6, 5, 11 | 1,000 | 1,000 | 1,000 |
| `duplicate_other_format` | 11 (4 senza tag) | min 0,978 (apostrofi) | 1,000 | 1,000 |
| `strip_tags` | 0 | - | - | - |

Per file `youtube_title` (normalizzato senza pulizia, poi pulito): `bradsucks-mess-02-copy` 0,579 / 1,0; `nin-ghosts-02-copy` 0,592 / 1,0; `nin-ghosts-25-copy` 0,605 / 1,0; `nin-slip-02-copy` 0,681 / 1,0; `nin-slip-08-copy` 0,609 / 1,0; `woodward-heretoday-01-copy` 0,629 / 1,0. Artista: Jaro-Winkler 1,0 in tutti i file che hanno il tag (anche `wrong_case`, grazie alla normalizzazione).

Effetto della pulizia sul punteggio finale (stessi pesi): la registrazione scelta non cambia (54/54 anche senza pulizia: AcoustID e durata bastano), ma i 6 file `youtube_title` scendono da 93,91-99,63 a **87,73-94,30**; senza pulizia `woodward-heretoday-01-copy` (87,73) finirebbe sotto la soglia di conferma in blocco. La pulizia serve alla confidenza, non alla scelta.

### 3.2 Durata

Differenza tra durata del file e durata della registrazione giusta: al massimo 1,75 s su tutti i 54 file (le più grandi sono di Josh Woodward: 1,67-1,75 s per `woodward-heretoday-01` e `-02`). Con la scala di 10 s la durata vale 0,825-0,999 per le proposte giuste. Nei casi ambigui del §2: "26 Ghosts III" dista 5,1 s (D = 0,49); le versioni strumentali distano meno di 1 s, quindi la durata non le separa bene e decidono titolo e O.

## 4. Pesi e soglie (Step 3)

### 4.1 Griglia

Valori provati (11.664 combinazioni in tutto): AcoustID {1, 2, 3}, durata {0, 1, 2}, titolo {0, 1, 2}, artista {0, 1}, titolo dell'album {0, 1}, edizione originale {0, 1, 2}, frazione di file del gruppo {0, 1, 2}, numero di tracce uguale {0, 1}; scala della durata {10 s, 30 s}; tag mancante {vale 0, vale 0,5, escluso dal punteggio}.

Ordine di scelta (lessicografico): (1) registrazioni giuste; (2) album giusti senza le copie da raccolta; (3) "copertura": proposte giuste con punteggio sopra il **candidato sbagliato più alto** di tutto il campione; (4) margine tra la proposta giusta più bassa e quel candidato; (5) pesi più piccoli.

"Candidato sbagliato" di un file = la migliore coppia registrazione + edizione del file con registrazione sbagliata oppure album sbagliato. È il punteggio che avrebbe la proposta se la risposta giusta mancasse da MusicBrainz, ed è quello che le soglie devono tenere sotto. Le registrazioni "quasi giuste" di Josh Woodward (2008 al posto di 2004, vedi il README della libreria) non contano come sbagliate; contandole come sbagliate (insieme alla seconda registrazione "Demon Seed" del bootleg, §2) il candidato sbagliato più alto resta 83,20.

### 4.2 Risultato

- Senza vincoli il migliore è **A 1, D 2, O 1** e tutto il resto 0 (scala 10 s): 54/54, 49/49, copertura 54/54, margine **15,69** (candidato sbagliato più alto 74,81). 340 combinazioni su 11.664 arrivano a 54/49/54.
- Il titolo e l'artista su questo campione non servono a scegliere: AcoustID trova sempre la registrazione e le ambiguità si risolvono con durata e O. Ma sono l'unico indizio contro una registrazione sbagliata con impronta simile (cover, karaoke, versione strumentale in un file senza informazioni di durata utili), un caso che il campione non contiene. Per questo la scelta finale impone titolo e artista almeno 1: il migliore è **A 1, D 2, T 1, R 1, O 1**, tag mancante escluso, scala 10 s: 54/49/54, margine **10,46** (86 combinazioni su 3.888 arrivano a 54/49/54). Costo: 5,2 punti di margine.
- Con il tag mancante che vale 0 o 0,5, nessuna combinazione con titolo e artista arriva alla copertura 54: i file senza tag scendono troppo. Per questo il tag mancante viene escluso dal punteggio.
- Con la scala della durata a 30 s 105 combinazioni arrivano a 54/49/54, ma tutte con margine più piccolo di quelle a 10 s.

### 4.3 Configurazioni di confronto

Stessa scala e stessa regola per i tag mancanti:

| Pesi (A, D, T, R, album, O, frazione, numero) | Registrazione giusta | di cui 2008 al posto di 2004 | Album giusto (49) | Edizione esatta (49) | Copertura | Candidato sbagliato più alto |
|---|---|---|---|---|---|---|
| Solo AcoustID (1,0,0,0,0,0,0,0) | 52/54 | 0 | 49 | 47 | 0 | 99,91 |
| AcoustID e durata (1,1,0,0,0,0,0,0) | 44/54 | 10 | 40 | 38 | 0 | 99,70 |
| Indizi del file, pesi uguali (1,1,1,1,0,0,0,0) | 44/54 | 10 | 40 | 38 | 0 | 99,85 |
| Tutti uguali (1,1,1,1,1,1,1,1) | 54/54 | 0 | 49 | 47 | 20 | 92,87 |
| **Scelta (1,2,1,1,0,1,0,0)** | **54/54** | 0 | **49** | **47** | **54** | **83,20** |

- Solo AcoustID sbaglia le 2 copie di `nin-ghosts-22` (vince "26 Ghosts III", che ha lo stesso score ed è elencata per prima) e non separa nulla: un candidato sbagliato (la stessa registrazione su una raccolta) ha lo stesso punteggio della proposta giusta.
- Senza O la durata porta 10 degli 11 file di Josh Woodward sulla registrazione del 2008 e sull'album "Here Today (Remixed)".
- Titolo dell'album, frazione di file e numero di tracce non cambiano nessuna scelta: nel campione ogni gruppo (tranne quello delle copie da raccolta) contiene file di una sola edizione, e il numero di tracce coincide solo con edizioni dello stesso album (tutte le edizioni di The Slip hanno 10 tracce). Aggiungerli abbassa solo il margine.

### 4.4 Controprova: un'edizione lasciata fuori

Pesi scelti sulla griglia con i file di 3 edizioni, provati sui file della quarta. "Soglia appresa" = candidato sbagliato più alto sui 3 edizioni di taratura.

Con titolo e artista imposti (la regola scelta):

| Edizione lasciata fuori | Pesi scelti | Registrazione giusta | Album giusto | Soglia appresa | Candidato sbagliato più alto nella prova | Candidati sbagliati sopra la soglia appresa |
|---|---|---|---|---|---|---|
| The Slip (17 file) | 1,2,1,1,0,1,0,0 | 17/17 | 15/15 | 83,17 | 83,20 | 2 (83,20 contro 83,17: stessa struttura, raccolta) |
| Ghosts I-IV (18 file) | 1,**0**,1,1,0,**2**,0,0 | 18/18 | 17/17 | 59,98 | **98,95** | 2 (`nin-ghosts-22`: "26 Ghosts III") |
| Guess Who's a Mess (8 file) | 1,2,1,1,0,1,0,0 | 8/8 | 7/7 | 83,20 | 82,84 | 0 |
| Here Today (11 file) | 1,2,1,1,0,1,0,0 | 11/11 | 10/10 | 83,20 | 82,72 | 0 |

Senza vincoli: stessi pesi del caso migliore (1,2,0,0,0,1,0,0) tranne Ghosts I-IV lasciata fuori, dove la griglia sceglie (1,0,0,0,0,2,0,0) e sbaglia 2 registrazioni su 18 (`nin-ghosts-22`); soglie apprese 74,75-74,81 contro candidati sbagliati fino a 74,81 nella prova.

Lettura: 3 edizioni su 4 danno gli stessi pesi; la quarta mostra che l'unico caso nel campione che richiede la durata è `nin-ghosts-22`. Con le soglie arrotondate a 85 e 90 (non al valore esatto appreso) nessun candidato sbagliato della prova supera la soglia nelle 3 edizioni con gli stessi pesi.

## 5. Soglie

Proposte del file (la scelta finale) e candidati sbagliati (il migliore per file), con i pesi scelti. "Al di sopra" vuol dire punteggio almeno uguale alla soglia.

| Soglia | Proposte sbagliate sopra | Candidati sbagliati sopra (35 file ne hanno) | Proposte giuste sotto (54) | Gruppi sbagliati sopra (1) | Edizioni sbagliate dei gruppi sopra | Gruppi giusti sotto (23) |
|---|---|---|---|---|---|---|
| 50 | 0 | 35 | 0 | 0 | 12 | 0 |
| 70 | 0 | 35 | 0 | 0 | 12 | 0 |
| 75 | 0 | 29 | 0 | 0 | 6 | 0 |
| 80 | 0 | 28 | 0 | 0 | 6 | 0 |
| **85** | 0 | **0** | 0 | 0 | **0** | 0 |
| **90** | 0 | 0 | **0** | 0 | 0 | **0** |
| 95 | 0 | 0 | 5 | 0 | 0 | 3 |

- Quasi tutti i candidati sbagliati sono **la registrazione giusta su un'altra edizione che non è l'album originale** (raccolte come "The Definitive Nine Inch Nails", il singolo "Echoplex", "Here Today (Remixed)", l'edizione strumentale di The Slip): con O = 0 il punteggio massimo è 100 * 5/6 = 83,3. Per questo il buco tra 83,3 e 93,66 è largo, e per questo le raccolte vere restano sempre sotto 85 (Decisione, punto 1).
- L'unico gruppo sbagliato è quello delle copie da raccolta: punteggio **39,87** (l'edizione migliore, The Slip, copre 2 file su 5), quindi "corrispondenza incerta", e Armonika propone di separare i file.
- Le proposte giuste più basse (93,66-94,42) sono 3 file di Josh Woodward (`-01-copy`, `-01-opus`, `-02-copy`, durata MusicBrainz lontana 1,67-1,75 s) e i 2 file di `nin-ghosts-36` senza tag (punteggio solo su A, D, O). A 95 questi cinque file e tre gruppi giusti finirebbero sotto la soglia di conferma in blocco senza togliere nessun errore nel campione.
- Zero sbagliati sopra soglia **è raggiunto** su questo campione, con 1,8 punti di margine per 85 e 6,8 per 90.

## 6. Per tipo di danno

Con i pesi scelti. Un file con più danni conta in ogni riga (es. `nin-slip-02-copy` è `youtube_title` e `strip_cover`). Le copie da raccolta sono confrontate con la ground truth così com'è (edizione originale), vedi §7.

| Danno | File | Registrazione giusta | Album giusto | Edizione esatta | Punteggio min-max | Candidato sbagliato più alto |
|---|---|---|---|---|---|---|
| clean | 11 | 11 | 11 | 11 | 96,14-99,82 | 83,17 |
| `youtube_title` | 6 | 6 | 6 | 5 | 93,91-99,63 | 82,96 |
| `strip_tags` | 5 | 5 | 5 | 4 | 94,42-99,35 | 74,35 |
| `strip_cover` | 5 | 5 | 5 | 5 | 98,53-99,62 | 82,96 |
| `split_album` | 6 | 6 | 6 | 6 | 98,64-99,83 | 83,17 |
| `wrong_case` | 5 | 5 | 5 | 5 | 93,66-99,87 | 83,20 |
| `duplicate_other_format` | 15 | 15 | 15 | 15 | 93,86-99,76 | 83,09 |
| `compilation_copy` | 5 | 5 | 5 | 4 | 96,14-99,81 | 79,92 |

- `youtube_title`: titolo "Artista - Titolo (Official Video) [HD]", senza album e numero. Registrazione 6/6; senza album la scelta dell'edizione viene da O e dalla data. Con la pulizia il titolo vale 1,0; senza pulizia 0,58-0,68 (§3.1).
- `strip_tags`: nessun tag. Registrazione 5/5 solo con AcoustID, durata e O; punteggi 94,42-99,35, candidati sbagliati fino a 74,35 (senza titolo e artista O pesa 1 su 4 invece di 1 su 6, quindi la stessa registrazione su una raccolta scende di più).
- `split_album`: il titolo dell'album "(Deluxe)" non disturba: il titolo dell'album ha peso 0, e i 5 file `-copy` di Ghosts I-IV formano comunque un gruppo coerente (punteggio del gruppo 99,17).

### 6.1 Casi falliti

Con i pesi scelti nessun file ha registrazione o album sbagliati. Restano:

1. **Edizione esatta, Brad Sucks** (`bradsucks-mess-02-copy`, `bradsucks-mess-04-copy`, `bradsucks-mess-05-compilation`): proposta `68e77602-36de-481d-b8f9-d42346cf93cc` invece di `e6577867-6876-4f54-98b1-da0db635bd51`. Sono due edizioni digitali dello stesso album, entrambe ufficiali, stessa data (2012-11-02), stesso numero di tracce (10); cambiano solo per l'apostrofo nel titolo. Nessun indizio le separa e la parità la decide l'ordine degli identificativi. Sono gruppi da un file (senza tag album o con album "Greatest Hits"); il gruppo di 5 file con album "Guess Who's a Mess" sceglie quella giusta perché `68e77602` non contiene tutte e 5 le registrazioni. Conseguenza per i doppioni, §8.
2. **Gruppo delle copie da raccolta**: punteggio 39,87, nessuna edizione comune (§7). È il comportamento voluto, non un errore, ma è un gruppo che non si può confermare così com'è.
3. **Fuori dalla scelta finale** (§4.3, §4.4): `nin-ghosts-22` (2 file) sbagliato senza il peso della durata; Josh Woodward (10 file) sulla registrazione del 2008 senza il peso di O.

## 7. Copie da raccolta

Le 5 copie da raccolta sono finte: hanno nei tag album "Greatest Hits" e artista dell'album "Various Artists", ma su MusicBrainz quella raccolta non esiste, e la ground truth dà come edizione quella originale. Come le ho trattate:

- **Registrazione**: valutata come per gli altri file: 5/5 giuste.
- **Edizione**: esclusa dai numeri principali (49 file) e riportata a parte: la proposta è sempre l'album originale (5/5 come album, 4/5 come edizione esatta, il quinto è il caso Brad Sucks del §6.1). È anche l'unica risposta possibile, perché la raccolta non esiste nel database.
- **Taratura**: nella griglia l'album giusto si conta solo sui 49 file non da raccolta; per le copie da raccolta un candidato su un altro album non conta come "candidato sbagliato" (conta solo una registrazione sbagliata). Così non spingono i pesi verso O né contro il titolo dell'album. Con questa regola il loro candidato sbagliato più alto è 79,92 (`nin-slip-10-compilation`).
- Trappola reale: per `nin-slip-10-compilation` MusicBrainz ha davvero un "Greatest Hits" (bootleg russo, 2008) con una registrazione "Demon Seed" con la stessa impronta. Un peso positivo sul titolo dell'album spingerebbe quel file verso il bootleg; con O il bootleg (stato `Bootleg`, tipo secondario `Compilation`) perde, e con i pesi scelti quel candidato vale 83,17, sotto 85.
- Le copie da raccolta diventano gruppi da un file dopo la separazione proposta, con punteggi 96,14-99,81, quindi singolarmente sopra la soglia di conferma in blocco.

## 8. Doppioni

Regola del design: stessa registrazione **e** stessa edizione nella stessa libreria. Dalle proposte (tutti i 54 file in una libreria):

- Stessa registrazione ed edizione esatta: **13 dei 15 doppioni** della ground truth (`duplicate_other_format`). Mancano `bradsucks-mess-02` e `bradsucks-mess-04`: la copia e il duplicato sono finiti su due edizioni digitali indistinguibili (§6.1, punto 1).
- Stessa registrazione e stesso album: 15 su 15.
- In entrambi i casi anche 4-5 **copie da raccolta risultano doppioni** del loro originale (4 con l'edizione esatta, 5 con l'album), mentre la ground truth dice `is_duplicate_of: null`. Non è un errore del riconoscimento: la raccolta finta non esiste su MusicBrainz, quindi registrazione ed edizione proposte sono quelle dell'originale, e la ground truth stessa dà a queste copie la stessa registrazione e la stessa edizione dell'originale. Con raccolte vere (presenti su MusicBrainz) il punto va rimisurato.

Da decidere in fase 2 (domanda per l'utente): se due edizioni dello stesso album non si distinguono da nessun indizio (stesso numero di tracce, stessa data, stesso formato), il controllo doppioni dovrebbe considerarle la stessa edizione, oppure il riconoscimento dovrebbe preferire l'edizione già presente nella libreria. Senza una delle due il doppione MP3 + FLAC dello stesso album può sfuggire.

## 9. Per la fase 2

- Rifare la misura con un campione più grande e vario: più artisti ed editori, file che vengono davvero da raccolte, singoli, live e remaster, file la cui registrazione non c'è su MusicBrainz, file con impronta che corrisponde a una registrazione sbagliata. Solo allora titolo dell'album, frazione di file e numero di tracce si possono tarare.
- Tenere la pulizia del titolo: senza, i titoli "stile YouTube" perdono 5-7 punti.
- Tenere i titoli MusicBrainz non puliti ("(instrumental)" è un'informazione).
- Valutare la regola su O del punto 1 e la soglia minima di AcoustID del punto 3 della Decisione.
- Per le edizioni dello stesso album: preferire quella già presente nella libreria, poi la data, poi il formato coerente con il file (digitale per un download).

## Riprodurre

Da Git Bash, nella radice del repository, con la libreria disordinata già generata (`bench/messy/README.md`):

```
cd spikes/04-recognition && cargo build --release && cd ../..
spikes/04-recognition/target/release/spike-recognition bench/data/messy bench/data/recognition/features.json

# Solo la prima volta (le risposte restano in bench/data/recognition-cache):
export ARMONIKA_ACOUSTID_KEY="$(powershell -NoProfile -Command "[Environment]::GetEnvironmentVariable('ARMONIKA_ACOUSTID_KEY','User')" | tr -d '\r\n')"
python spikes/04-recognition/fetch.py bench/data/recognition/features.json bench/data/recognition-cache

python spikes/04-recognition/recognize.py bench/data/recognition/features.json \
    bench/data/recognition-cache bench/data/messy/ground_truth.json \
    bench/messy/manifest.toml bench/data/recognition/results.json
```

`recognize.py` richiede numpy (provato con 1.24.3) e Python 3.11 (`tomllib`); gira in pochi secondi. Tutti i numeri di questo documento sono in `bench/data/recognition/results.json` (chiavi `with_text`, `unconstrained`, `refs`, `loeo`, `thresholds`, `per_file`, `groups_final`, `duplicates`, `candidates`, `no_cleanup`).
