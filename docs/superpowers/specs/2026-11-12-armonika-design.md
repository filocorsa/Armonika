# Armonika: documento di design

Data: 2026-11-12
Stato: bozza da approvare

## 1. Cos'è e perché

Armonika è un server musicale self-hosted, scritto in Rust e rilasciato open source con licenza AGPL-3.0.

Lo spazio è affollato: Navidrome, gonic, Jellyfin, Ampache, Polaris (che è già in Rust) e altri. "È in Rust e consuma poco" da solo non basta, perché Navidrome consuma già poco. Il motivo per scegliere Armonika è un altro:

> **Armonika è il server musicale leggero che mette in ordine la tua libreria.**

Nel self-hosting musicale la difficoltà vera non sta nel far girare il server, ma nell'organizzare la libreria. I server leggeri (Navidrome, gonic, LMS, Polaris) non scrivono i tag e si aspettano una libreria già ordinata. Quelli che li scrivono (Jellyfin, Plex, Ampache) sono pesanti o generalisti. Oggi l'ordine si fa a parte, con beets (da riga di comando) o MusicBrainz Picard (da desktop). Armonika unisce le due cose: streaming veloce e riordino della libreria dal browser.

Il secondo elemento distintivo è l'**integrazione opzionale con MeTube**. Dall'interfaccia di Armonika incolli un link, MeTube scarica il file, e Armonika lo riconosce, lo sistema e lo aggiunge alla libreria senza che l'utente debba aprire MeTube.

Fonti: https://github.com/basings/selfhosted-music-overview (confronto tra server, versione letta il 2026-11-12).

## 2. Obiettivi

1. **Ascolto veloce prima di tutto.** Scansione, ricerca istantanea e avvio rapido dello streaming sono le fondamenta: niente altro viene prima.
2. **Compatibilità con l'ecosistema esistente** tramite l'API OpenSubsonic. Dal primo giorno funzionano le app già esistenti (Symfonium, Feishin, Substreamer, Tempo e altre).
3. **Riordino della libreria**: una coda "da sistemare", proposte da MusicBrainz, correzione e scrittura dei tag sui file, rinomina e spostamento dei file in una struttura ordinata. Tutto con conferma e con possibilità di annullare.
4. **Integrazione MeTube** opzionale, gestita interamente da Armonika.
5. **Interfaccia web moderna**, nettamente migliore di Navidrome e simili sia nell'aspetto sia nell'uso.
6. **Facile da installare**: Docker + un docker-compose d'esempio, oppure un unico eseguibile.

## 3. Obiettivi di velocità (misurati, non promessi)

Sono **criteri di accettazione**: un cambiamento che li peggiora oltre la soglia non entra.

| Cosa | Soglia | Condizioni |
|---|---|---|
| Ricerca "mentre scrivi" | < 50 ms (p95) lato server | 100.000 brani, con tolleranza a 1 errore di battitura |
| Avvio streaming | < 200 ms al primo byte audio | rete di casa, file originale senza transcodifica |
| Prima scansione | 50.000 brani in pochi minuti (obiettivo indicativo: < 5 min) | disco HDD normale |
| Scansioni successive | solo i file cambiati (mtime/dimensione) | nessuna |
| Risorse a riposo | indicativo: < 100 MB di RAM con 100k brani | vecchio PC da homelab |

Le soglie si verificano con una suite di benchmark ripetibile (vedi §10).

## 4. Non-obiettivi (almeno per ora)

- **App native** (mobile/desktop). Ci pensano le app Subsonic esistenti.
- **Download diretto da YouTube.** Armonika non include yt-dlp e non scarica nulla da solo: parla con MeTube solo se l'utente ce l'ha. Questo protegge il progetto (nel 2020 youtube-dl è stato rimosso da GitHub dopo una segnalazione DMCA).
- **Multi-room / riproduzione sincronizzata** su più stanze.
- **DLNA, jukebox mode, video, podcast, audiolibri.**
- **Ascolto di gruppo** (sessione con musica e coda condivise tra più persone): non ora, ma il design non deve impedirlo (§8.5).
- **Link pubblici di condivisione** per persone senza account: non ora.
- **Player web nella prima fase**: arriva dopo l'interfaccia di gestione (vedi §9).

## 5. Anti-obiettivi (cose che lo renderebbero un fallimento)

- **Progetto strutturato male**, che dopo mesi di lavoro si rivela inutilizzabile o da rifare. Contromisure obbligatorie:
  - ogni fase si chiude con qualcosa che si usa davvero a casa;
  - le parti rischiose si verificano prima con piccoli test usa-e-getta (spike, §11);
  - i numeri di velocità sono misurati automaticamente.
- **Rovinare i file musicali dell'utente.** Mai modificare o spostare un file senza conferma esplicita; prima di ogni modifica si salva lo stato precedente (tag originali e percorso originale) in un registro che permette di annullare.
- **Setup che scoraggia.** Nessun passaggio di configurazione in più se non è indispensabile (es. MeTube funziona con il solo indirizzo, senza cartelle condivise).
- **Diventare "solo un Navidrome in Rust"**: se il riordino della libreria non è usabile, Armonika non ha motivo di esistere.

## 6. Decisioni prese con l'utente

- **Licenza: AGPL-3.0, senza CLA.**
- **Multi-utente fin dall'inizio, con librerie e permessi.** Possono esistere più librerie; per ognuna l'admin decide chi accede e con quale permesso (ascolto o gestione). Una libreria condivisa e una personale sono lo stesso meccanismo. Chi ha il permesso di gestione su una libreria può aggiungere e sistemare musica in quella libreria. Playlist private di default, pubblicabili in sola lettura. Ascolti contemporanei senza limiti, ognuno per conto suo. Dettagli completi in §8.5.
- **Due API**: OpenSubsonic (compatibilità) e un'API nativa di Armonika (tutto ciò che Subsonic non sa esprimere, e base dell'interfaccia web).
- **Interfaccia web di gestione presto** (fase 2), player web dopo.
- **Modalità headless**: l'interfaccia web si può disattivare dalla configurazione; restano le API.
- **MeTube**: opzionale; di default basta l'indirizzo dell'istanza e Armonika recupera il file via rete; la cartella condivisa è solo un'opzione per chi ce l'ha già.
- **Ordine delle fasi**: prima le fondamenta di ascolto, poi riordino e interfaccia, poi MeTube, infine il player web.

## 7. Scelte tecniche (decise da Claude, contestabili)

### 7.1 Forma del prodotto
- **Un unico eseguibile** Rust che contiene server, entrambe le API e l'interfaccia web (i file statici sono incorporati nel binario).
- **Multipiattaforma** (requisito dell'utente). Due modi di installazione, entrambi supportati allo stesso livello:
  - **Docker**: immagine multi-architettura pubblicata su GitHub Container Registry (linux/amd64 e linux/arm64, per PC, NAS e Raspberry Pi), con un docker-compose d'esempio pronto da copiare.
  - **Binari** su GitHub Releases per Linux (x86_64 e arm64), Windows (x86_64) e macOS (Intel e Apple Silicon).
- Conseguenze sul codice: niente percorsi scritti a mano con `/` o `\` (si usano le API di Rust per i percorsi), attenzione a maiuscole/minuscole nei nomi dei file (Windows e macOS non le distinguono, Linux sì) e ai caratteri non ammessi nei nomi dei file su Windows quando Armonika rinomina i brani. ffmpeg è incluso nell'immagine Docker; per i binari è una dipendenza opzionale, serve solo per la transcodifica.
- La CI compila e fa girare i test su Linux, Windows e macOS a ogni modifica, non solo al momento del rilascio.
- Configurazione: file TOML + variabili d'ambiente (`ARMONIKA_*`), con default sensati; la maggior parte delle impostazioni modificabile anche dall'interfaccia.
- Cartelle: una o più cartelle di musica (una per libreria, §8.5), una cartella `inbox` per ogni libreria (file da sistemare), `data` (database, cache copertine, indice di ricerca). Nel caso più semplice c'è una sola libreria con le cartelle `music` e `inbox`.

### 7.2 Stack backend
- Runtime async **Tokio**, server HTTP **axum**.
- Database **SQLite** in modalità WAL (un solo scrittore, molti lettori: adatto a questo uso). Migrazioni versionate fin dal primo giorno.
- Lettura/scrittura tag con **lofty** (MP3/ID3, FLAC, Ogg Vorbis, Opus, M4A/AAC, WAV/AIFF).
- Transcodifica opzionale tramite **ffmpeg** esterno (presente nell'immagine Docker). Di default i file si inviano così come sono, con supporto ai range HTTP per l'avanzamento nel brano.
- Riconoscimento brani: **MusicBrainz** (ricerca per metadati) e **AcoustID** (impronta audio tramite chromaprint) per i file con tag pessimi, come quelli da YouTube. Va rispettato il limite di MusicBrainz (1 richiesta al secondo, con User-Agent identificativo del progetto). AcoustID richiede una chiave applicazione: il progetto ne registra una propria, sostituibile dall'utente nelle impostazioni. Il riconoscimento richiede internet; senza connessione Armonika continua a funzionare e la coda accetta solo correzioni manuali.
- API nativa REST/JSON con specifica **OpenAPI** generata dal codice; aggiornamenti in tempo reale (scansione, download MeTube, coda) via **Server-Sent Events**.

### 7.3 Ricerca
- I dati stanno in SQLite; la ricerca è isolata dietro un'interfaccia propria (`SearchIndex`), così il motore si può sostituire senza toccare il resto.
- Prima scelta: **SQLite FTS5**. Se lo spike (§11) mostra che non regge la soglia con gli errori di battitura, si passa a **Tantivy** (motore di ricerca Rust incorporato). La decisione si prende con lo spike, prima della fase 1.

### 7.4 Utenti e autenticazione
- L'autenticazione Subsonic classica (token = md5(password + salt)) obbliga il server a conoscere la password in chiaro: le password vanno quindi cifrate in modo reversibile con una chiave locale, come fa Navidrome. Si supporta anche l'estensione **API key** di OpenSubsonic.
- **Primo admin** (serve già in fase 1, quando non c'è ancora l'interfaccia): creato al primo avvio da variabili d'ambiente (`ARMONIKA_ADMIN_USER` / `ARMONIKA_ADMIN_PASSWORD`) oppure con un comando dell'eseguibile (`armonika user create --admin`). Gli altri utenti in fase 1 si creano con lo stesso comando; dalla fase 2 anche dall'interfaccia. *(Scelta di Claude.)*
- L'API nativa e l'interfaccia web usano sessioni o token propri, con le password salvate in modo non reversibile (argon2). In pratica: password dell'account per il web, e per le app Subsonic una password o API key separata, generata dall'interfaccia. *(Dettaglio esatto lasciato al builder, purché le app Subsonic funzionino con utente e password.)*

### 7.5 Struttura del codice
Un'unica repository (monorepo) con un workspace Cargo e l'interfaccia web a parte:

```
armonika/
  crates/
    armonika-core/      modello di dominio, database, migrazioni
    armonika-scanner/   scansione e lettura tag
    armonika-search/    SearchIndex e implementazioni
    armonika-organizer/ coda "da sistemare", MusicBrainz/AcoustID, scrittura tag, registro annullamenti
    armonika-subsonic/  API OpenSubsonic
    armonika-api/       API nativa
    armonika-metube/    client MeTube
    armonika-server/    eseguibile: config, avvio, wiring, incorpora la UI
  web/                  interfaccia web
  bench/                generatore di librerie finte e benchmark
  docs/
```

Regola: le crate API dipendono dal core, mai il contrario; l'integrazione MeTube è isolata e disattivabile. *(La suddivisione esatta può essere semplificata dal builder se all'inizio alcune crate sono troppo piccole; vincolanti sono i confini, non il numero di crate.)*

### 7.6 Interfaccia web
- **Svelte 5 + TypeScript + Vite**, compilata in file statici incorporati nel binario.
- Client dell'API generato dalla specifica OpenAPI.
- Brand: colori e logo da `images/armonika-brand-kit/BRAND.md` (sfondo scuro `#0F1115`, unico accento ruggine `#C2452D`, testo crema `#F4F1EC`). Tema scuro per primo.
- Lingue: inglese e italiano, con struttura pronta per altre traduzioni.

### 7.7 Lingua del progetto
Codice, commenti, README, documentazione pubblica e messaggi di commit in **inglese**, perché il progetto è open e internazionale. I documenti interni di pianificazione possono restare in italiano.

Stile dei testi: niente segni tipici del testo generato da AI (lineette lunghe, frecce, carattere dei puntini di sospensione, virgolette tipografiche, titoli pieni di emoji). Si usa punteggiatura semplice. Vale per documentazione, commenti nel codice, commit, PR e testi dell'interfaccia.

## 8. Comportamenti chiave

### 8.1 I file sono la verità
Il database si può sempre ricostruire dai file musicali. I dati che non stanno nei file (utenti, preferiti, playlist, cronologia) sono esportabili. Brani e album hanno identificativi stabili, così uno spostamento o una rinomina fatti da Armonika non fanno perdere preferiti o playlist.

### 8.2 La coda "da sistemare"
- In coda finiscono: i file nella cartella `inbox`, i download completati da MeTube e, su richiesta, i file della libreria con problemi (tag mancanti, copertina assente, album spezzati o duplicati).
- Per ogni elemento Armonika propone una corrispondenza (MusicBrainz/AcoustID) con un **grado di confidenza**, i tag proposti e la copertina, mostrando le differenze rispetto a quelli attuali.
- Ogni libreria ha la sua coda. La vede e la gestisce solo chi ha il permesso di gestione su quella libreria.
- Chi ha il permesso di gestione conferma, corregge o scarta. Alla conferma: si salva lo stato precedente nel registro, si scrivono i tag, si sposta e rinomina il file secondo uno schema configurabile (default: `Artista Album/Album (Anno)/NN - Titolo.ext`) e si aggiorna l'indice.
- **Annulla**: ogni operazione nel registro si può annullare (tag e percorso originali).
- Applicazione automatica sopra una certa confidenza: **non nella prima versione**; il design la deve rendere possibile in seguito.

### 8.3 Scrittura sulla libreria esistente
Per default Armonika **non scrive** nei file già presenti in una libreria (la cartella può essere montata in sola lettura). Scrivere tag o spostare file già presenti è un'opzione che l'admin attiva esplicitamente, libreria per libreria. I file importati dalla `inbox` vengono invece sempre spostati nella libreria, perché è lo scopo dell'importazione.

### 8.4 MeTube
- Impostazione (solo admin): indirizzo dell'istanza MeTube (ed eventuali credenziali). Senza, la funzione non appare.
- "Aggiungi da link" è disponibile solo a chi ha il permesso di gestione su almeno una libreria. Si sceglie la libreria di destinazione tra quelle in cui si ha il permesso, e il file finisce nella coda di quella libreria.
- Dall'interfaccia si usa il campo "Aggiungi da link". Armonika invia la richiesta a MeTube (formato audio) e mostra lo stato del download (in coda, in corso, completato, fallito). Al completamento recupera il file via HTTP da MeTube, e il file entra nella coda "da sistemare".
- Opzionale: se la cartella dei download di MeTube è montata anche in Armonika, il file si legge da lì senza scaricarlo.
- Formato richiesto a MeTube: audio nella migliore qualità disponibile, senza riconversioni inutili (di solito Opus o M4A), con la copertina incorporata se MeTube lo consente. Scelta modificabile nelle impostazioni. *(Scelta di Claude.)*
- Il formato di MeTube non è uno standard stabile: il client va isolato e coperto da test contro una vera istanza MeTube (in Docker).

### 8.5 Utenti, librerie e permessi

Questa sezione definisce in modo preciso cosa significa "multi-utente". Le scelte marcate *(decisa con l'utente)* non si cambiano senza tornare da lui; le altre sono scelte di Claude.

**Ruoli dell'account**
- **Admin del server**: crea e gestisce utenti e librerie, assegna i permessi, gestisce le impostazioni del server e il collegamento a MeTube. Ha automaticamente il permesso di gestione su tutte le librerie. Possono esserci più admin; non si può eliminare o declassare l'ultimo admin.
- **Utente**: accede solo alle librerie per cui ha un permesso. Non vede le impostazioni del server.

**Librerie** *(decisa con l'utente)*
- Una libreria è una cartella di musica sul server, con un nome. Possono essercene quante se ne vuole.
- Le cartelle di due librerie **non possono sovrapporsi** (nessuna libreria dentro un'altra), così ogni file appartiene a una sola libreria.
- Lo stesso album presente in due librerie è fatto di file diversi e viene trattato come due album diversi. Non si cerca di unirli.
- "Libreria condivisa" e "libreria personale" non sono tipi diversi: sono librerie con più utenti o con un solo utente ammesso.

**Permessi per libreria** *(decisa con l'utente)*
- **Ascolto**: vedere, cercare e ascoltare la musica della libreria.
- **Gestione**: ascolto, più aggiungere musica (inbox, MeTube), usare la coda "da sistemare", modificare i tag, rinominare e spostare file dentro la libreria (se la scrittura è attiva, §8.3), annullare le operazioni del registro.
- Eliminare un file non lo cancella subito: lo sposta in un cestino della libreria, svuotabile solo dall'admin.
- Spostare musica da una libreria a un'altra richiede il permesso di gestione su entrambe.
- Senza permesso, una libreria è invisibile: non compare nella ricerca, nella navigazione, nelle API né nei conteggi.

**Dati personali di ogni utente**
- Preferiti, valutazioni, numero di ascolti, cronologia, playlist, coda di riproduzione salvata, collegamenti a Last.fm e ListenBrainz, preferenze (es. qualità della transcodifica), credenziali per le app Subsonic.
- Sono sempre per utente: il "numero di ascolti" di un brano è il tuo, non quello di tutta la famiglia.

**Playlist** *(decisa con l'utente)*
- Private di default. Il proprietario può renderle **pubbliche**: gli altri utenti le vedono e le ascoltano, ma non le modificano.
- Una playlist può contenere brani di più librerie. Chi la guarda vede solo i brani delle librerie a cui ha accesso; gli altri sono nascosti, non mostrati come errori.

**Ascolto contemporaneo** *(decisa con l'utente)*
- Nessun limite al numero di utenti e dispositivi che ascoltano insieme; ognuno ascolta la sua musica in modo indipendente.
- Lo stesso utente può usare più dispositivi. La coda di riproduzione salvata permette di riprendere su un altro dispositivo (funzione Subsonic `savePlayQueue`/`getPlayQueue`); se due dispositivi la salvano, vale l'ultimo.
- "Cosa stanno ascoltando gli altri" (funzione Subsonic `getNowPlaying`): per privacy ogni utente vede solo il proprio ascolto, a meno che non scelga di renderlo visibile agli altri.
- L'ascolto di gruppo non si costruisce ora; il design non deve impedirlo in futuro (es. non legare la coda di riproduzione a un singolo dispositivo in modo rigido).

**Cambi di permessi e cancellazioni**
- Se un utente perde l'accesso a una libreria, i suoi preferiti, valutazioni e cronologia su quei brani restano salvati ma diventano invisibili; tornano se l'accesso viene ridato.
- Se si elimina un utente, si cancellano i suoi dati personali e le sue playlist (anche quelle pubbliche). I file musicali non vengono mai toccati.
- Se si elimina una libreria da Armonika, si rimuovono i dati dal database ma **non i file** sul disco.

**Come si vede dalle app Subsonic**
- Ogni libreria corrisponde a una "cartella musicale" Subsonic (`getMusicFolders`). Ogni utente vede solo le proprie, e le app che lo supportano permettono di filtrare per libreria.

**Cosa c'è in ogni fase**
- Fase 1: il modello completo (librerie, permessi, dati per utente) esiste già nel database e nelle API Subsonic. Librerie e permessi si gestiscono dal file di configurazione o con comandi dell'eseguibile.
- Fase 2: gestione di utenti, librerie e permessi dall'interfaccia web.

## 9. Fasi

Ogni fase si chiude solo quando il suo **criterio di uscita** è vero.

**Fase 0: Spike e impalcatura.** Gli spike del §11, la struttura della repository, CI (build, test, lint, benchmark), generatore di librerie finte.
*Uscita:* motore di ricerca scelto con dati alla mano; recupero dei file da MeTube verificato; CI verde.

**Fase 1: Fondamenta di ascolto.** Scansione (completa + incrementale + osservazione delle cartelle), database, ricerca, streaming con range HTTP, copertine, utenti, ruoli, librerie e permessi (§8.5), API OpenSubsonic (navigazione, ricerca, streaming, preferiti, playlist, scrobbling Last.fm/ListenBrainz), modalità headless, Docker.
*Uscita:* l'utente usa Armonika ogni giorno sul suo homelab con almeno due app Subsonic (es. Symfonium e Feishin), e i benchmark rispettano le soglie del §3.

**Fase 2: Il cuore di Armonika.** API nativa, interfaccia web di gestione (accesso, utenti, librerie e permessi, impostazioni, stato della scansione, coda "da sistemare", modifica dei tag, registro e annullamento), MusicBrainz + AcoustID, scrittura dei tag, rinomina e spostamento.
*Uscita:* partendo dalla libreria "disordinata apposta" (§10), l'utente la mette in ordine interamente dal browser, e ogni operazione si può annullare.

**Fase 3: MeTube.** Collegamento, "Aggiungi da link", stato dei download, recupero del file, ingresso nella coda.
*Uscita:* dal divano, si incolla un link in Armonika e il brano arriva sistemato e ascoltabile da un'app Subsonic, senza aprire MeTube.

**Fase 4: Player web.** Riproduzione nel browser dentro la stessa interfaccia.

La repository resta **privata** durante lo sviluppo. Il rilascio pubblico (apertura della repository e prima release) è previsto indicativamente alla fine della fase 2, quando la caratteristica distintiva esiste; il momento lo decide l'utente. Siccome all'apertura diventa pubblica tutta la cronologia git, nessun segreto (chiavi, password, file personali) deve mai essere committato, nemmeno temporaneamente.

Nei commit, nelle PR e nei file del progetto non va indicato Claude come autore o co-autore.

## 10. Dati di prova

L'utente non ha ancora una libreria musicale, quindi:
- **Libreria grande finta**: generatore che crea centinaia di migliaia di file audio minuscoli (pochi ms di silenzio) con tag realistici e vari. Serve per i benchmark.
- **Libreria "disordinata apposta"**: brani con licenza Creative Commons (es. Free Music Archive, Jamendo) con tag rovinati di proposito: titoli stile YouTube, tag mancanti, album spezzati, duplicati, copertine assenti, formati diversi. Serve per testare il riordino. Va documentata la provenienza e la licenza di ogni file.

## 11. Spike da fare per primi (usa-e-getta)

1. **Ricerca**: FTS5 contro Tantivy su 100k brani con errori di battitura, per scegliere il motore.
2. **MeTube**: verificare come si invia un download, come si legge lo stato e se il file finito si recupera in modo affidabile via HTTP.
3. **Scrittura tag**: lofty su tutti i formati elencati, controllando che i file restino leggibili dalle app comuni e che non si perdano dati (copertine incorporate, tag sconosciuti).
4. **Riconoscimento**: quanti brani "stile YouTube" AcoustID + MusicBrainz riconoscono correttamente.

Gli spike non diventano codice di produzione: ciò che si impara entra nel piano.

## 12. Lasciato al builder

- Scelte di libreria specifiche non fissate qui (es. sqlx o rusqlite, osservazione delle cartelle, generazione OpenAPI).
- Schema esatto del database, purché rispetti §6 e §8 (dati per utente, identificativi stabili, registro annullamenti, librerie e permessi come in §8.5).
- Design visivo dell'interfaccia, nel rispetto del brand kit.
- Dettaglio dell'autenticazione (§7.4), con il vincolo di compatibilità Subsonic.
- Quali endpoint OpenSubsonic implementare per primi, in base a ciò che usano Symfonium e Feishin.
