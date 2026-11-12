# Armonika: documento di design

Data: 2026-11-12
Stato: approvato dall'utente il 2026-11-12

## Glossario

Queste parole hanno **un solo significato** in tutto il documento, nel codice, nell'API nativa e nell'interfaccia. Se un testo usa una parola in modo diverso, è un errore da correggere. *(Glossario approvato dall'utente.)*

| Termine | Significato | Esempio |
|---|---|---|
| **File** | L'oggetto fisico sul disco: percorso, formato, qualità (bitrate, frequenza), dimensione, tag scritti dentro. | `Queen/A Night at the Opera (1975)/11 - Bohemian Rhapsody.flac` |
| **Traccia** | La voce che Armonika mostra, cerca e riproduce in una libreria (nelle app Subsonic si chiama "song"). Ha **un file alla volta**, ma la sua identità non coincide con il file: se il file viene spostato, rinominato, ritaggato o sostituito, la traccia resta la stessa, con lo stesso identificativo. Esiste solo dentro una libreria. | "Bohemian Rhapsody" nella libreria Famiglia, con il suo identificativo |
| **Registrazione** | Una specifica esecuzione o versione, come la intende MusicBrainz ("recording"). Più tracce, anche in edizioni diverse, possono avere la stessa registrazione. | Studio 1975, live 1986 a Mannheim, mix 5.1 del 2001: tre registrazioni diverse |
| **Canzone** | L'opera in astratto, che comprende tutte le registrazioni e le cover (MusicBrainz la chiama "work"). Per ora è solo un'informazione. | "Bohemian Rhapsody" come composizione |
| **Edizione** | Una specifica pubblicazione di un disco ("release" in MusicBrainz): elenco tracce, anno, etichetta, formato. | "A Night at the Opera", remaster deluxe 2011 |
| **Album** | Il disco in generale, che raggruppa tutte le sue edizioni ("release group" in MusicBrainz). | "A Night at the Opera" (tutte le edizioni) |
| **Album della libreria** | L'insieme delle tracce di una stessa edizione dentro una stessa libreria. È ciò che l'utente vede come "album" nelle app. Ha un identificativo stabile come la traccia (regole in §8.1). | L'edizione 2011 di "A Night at the Opera" nella libreria Famiglia |
| **Artista** | Un'entità, non una scritta: identificata dall'identificativo MusicBrainz se noto, altrimenti dal nome normalizzato. Ha un identificativo stabile. | Queen; David Bowie |
| **Accredito** | La scritta che indica chi suona una traccia o un album, che può nominare più artisti. Una traccia compare sotto ogni artista del suo accredito; i preferiti si mettono sull'artista, non sull'accredito. | "Queen & David Bowie" (accredito con due artisti) |
| **Artista della traccia / artista dell'album** | L'accredito di una traccia può essere diverso da quello del suo album della libreria (es. raccolte, featuring). | Traccia: "Queen & David Bowie"; album della libreria: "Queen" |
| **Libreria** | Una cartella di musica sul server, con un nome, i suoi permessi, la sua coda e il suo cestino (§8.5). | "Famiglia", "Marco" |
| **Elemento in coda** | Un file appena arrivato (browser, MeTube, `inbox`) che aspetta di essere confermato. **Non è ancora una traccia**: non compare nella libreria né nelle app. | Il file scaricato da un link YouTube |
| **Gruppo** | Un insieme di elementi in coda che si rivedono e si confermano insieme. Di solito corrisponde a un'edizione; un elemento isolato è un gruppo da uno. Riguarda solo la coda. | I 12 file di una stessa edizione caricati in uno zip |
| **Revisione di tracce** | La correzione di tracce **già presenti** in una libreria (tag mancanti, copertina assente, album della libreria spezzati, doppioni). Usa lo stesso schermo della coda ma lavora su tracce, non su elementi in coda, e richiede la scrittura attiva (§8.3). | Sistemare un album già in libreria con le tracce senza anno |
| **Doppione** | Una traccia, o un elemento in coda, che ha la **stessa registrazione e la stessa edizione** di una traccia già presente **nella stessa libreria**. Definizione completa in §8.6. | La stessa edizione caricata prima in MP3 e poi in FLAC |
| **Cestino** | Il posto, dentro ogni libreria, dove finiscono i file eliminati o scartati prima della cancellazione definitiva (§8.5). | |

Regole che discendono dal glossario:
- **I dati musicali di una traccia (titolo, accrediti, album, numero, anno, copertina e così via) sono sempre e solo i tag del suo file.** Armonika non tiene correzioni proprie nel database sopra i tag: per correggere una traccia si scrivono i tag nel file. Nelle librerie in sola lettura le tracce non si possono correggere, e l'interfaccia spiega che serve attivare la scrittura (§8.3). *(Decisa con l'utente.)*
- **Playlist, preferiti, valutazioni, numero di ascolti, cronologia e download offline sono legati alla traccia**, mai al file né alla registrazione.
- Una playlist può contenere la stessa canzone in registrazioni diverse, e anche la stessa traccia più volte: è sempre lecito e nessuno lo segnala.
- Lo scrobbling (Last.fm, ListenBrainz) comunica la registrazione della traccia ascoltata, se nota; altrimenti i tag del file.

## 1. Cos'è e perché

Armonika è un server musicale self-hosted, scritto in Rust e rilasciato open source con licenza AGPL-3.0.

Lo spazio è affollato: Navidrome, gonic, Jellyfin, Ampache, Polaris (che è già in Rust) e altri. "È in Rust e consuma poco" da solo non basta, perché Navidrome consuma già poco. Il motivo per scegliere Armonika è un altro:

> **Armonika è il server musicale leggero che mette in ordine la tua libreria.**

Nel self-hosting musicale la difficoltà vera non sta nel far girare il server, ma nell'organizzare la libreria. I server leggeri (Navidrome, gonic, LMS, Polaris) non scrivono i tag e si aspettano una libreria già ordinata. Quelli che li scrivono (Jellyfin, Plex, Ampache) sono pesanti o generalisti. Oggi l'ordine si fa a parte, con beets (da riga di comando) o MusicBrainz Picard (da desktop). Armonika unisce le due cose: streaming veloce e riordino della libreria dal browser.

Il secondo elemento distintivo è l'**integrazione opzionale con MeTube**. Dall'interfaccia di Armonika incolli un link, MeTube scarica il file, e Armonika ne riconosce la registrazione, propone i tag giusti e, dopo la conferma, lo aggiunge alla libreria come nuova traccia, senza che l'utente debba aprire MeTube.

Fonti: https://github.com/basings/selfhosted-music-overview (confronto tra server, versione letta il 2026-11-12).

Come gli altri server gestiscono il caricamento della musica (ricerca del 2026-11-12):
- **Nessun caricamento dall'app**: Navidrome (per scelta: la cartella della musica è in sola lettura, si consiglia un file manager esterno), Jellyfin (solo con plugin di terze parti), Plex (richiesto dagli utenti da anni), gonic, LMS, Polaris.
- **Caricamento dall'interfaccia web**: Koel (solo admin), Ampache (dalla versione 7 libreria di destinazione per utente; chi carica può modificare i propri caricamenti), mStream (permesso per utente dalla v6), Funkwhale (tutti, con spazio massimo per utente, default 1 GB), Airsonic-Advanced (verso cartelle prestabilite; vulnerabilità pubblicata nel 2025, vedi §8.5).
- Nessuno combina caricamento, riordino dei tag e coda di revisione: è lo spazio di Armonika.

Un precedente da non ripetere: Navidrome ha dovuto introdurre identificativi persistenti dopo il rilascio, perché gli utenti perdevano preferiti e playlist quando ritaggavano o spostavano i file. In Armonika la separazione tra traccia e file (Glossario) esiste dal primo giorno.

## 2. Obiettivi

1. **Ascolto veloce prima di tutto.** Scansione, ricerca istantanea e avvio rapido dello streaming sono le fondamenta: niente altro viene prima.
2. **Compatibilità con l'ecosistema esistente** tramite l'API OpenSubsonic. Dal primo giorno funzionano le app già esistenti (Symfonium, Feishin, Substreamer, Tempo e altre).
3. **Riordino della libreria**: una coda "da sistemare", riconoscimento di registrazioni ed edizioni tramite MusicBrainz, correzione e scrittura dei tag nei file, rinomina e spostamento dei file in una struttura ordinata. Tutto con conferma e con possibilità di annullare.
4. **Integrazione MeTube** opzionale, gestita interamente da Armonika.
5. **Interfaccia web moderna**, nettamente migliore di Navidrome e simili sia nell'aspetto sia nell'uso.
6. **Facile da installare**: Docker + un docker-compose d'esempio, oppure un unico eseguibile.

## 3. Obiettivi di velocità (misurati, non promessi)

Sono **criteri di accettazione**: un cambiamento che li peggiora oltre la soglia non entra.

| Cosa | Soglia | Condizioni |
|---|---|---|
| Ricerca "mentre scrivi" | < 50 ms (p95) lato server | 100.000 tracce, con tolleranza a 1 errore di battitura |
| Avvio streaming | < 200 ms al primo byte audio | rete di casa, file originale senza transcodifica |
| Prima scansione | 50.000 file in pochi minuti (obiettivo indicativo: < 5 min) | disco HDD normale |
| Scansioni successive | si rileggono solo i file cambiati (data di modifica/dimensione) | nessuna |
| Risorse a riposo | indicativo: < 100 MB di RAM con 100.000 tracce | vecchio PC da homelab |

Le soglie si verificano con una suite di benchmark ripetibile (vedi §10).

## 4. Non-obiettivi (almeno per ora)

- **App native** (mobile/desktop). Ci pensano le app Subsonic esistenti.
- **Download diretto da YouTube.** Armonika non include yt-dlp e non scarica nulla da solo: parla con MeTube solo se l'utente ce l'ha. Questo protegge il progetto (nel 2020 youtube-dl è stato rimosso da GitHub dopo una segnalazione DMCA).
- **Multi-room / riproduzione sincronizzata** su più stanze.
- **DLNA, jukebox mode, video, podcast, audiolibri.**
- **Ascolto di gruppo** (sessione con musica e coda condivise tra più persone): non ora, ma il design non deve impedirlo (§8.5).
- **Link pubblici di condivisione** per persone senza account: non ora.
- **Armonika come servizio pubblico** *(decisa con l'utente)*: il target è il self-hosting per sé, la famiglia e gli amici. Non si costruiscono iscrizione libera, verifica email per sconosciuti, piani, pagamenti, gestione delle segnalazioni di copyright. Il design però non lo impedisce: i limiti di spazio (§8.5) sono generici, così chi vuole offrire un servizio può costruirci sopra i propri piani (la licenza AGPL lo consente, con l'obbligo di pubblicare le modifiche).
- **Funzioni basate sulla canzone** (es. "mostra tutte le registrazioni di questa canzone"): non ora; il dato si conserva quando MusicBrainz lo fornisce.
- **Player web nella prima fase**: arriva dopo l'interfaccia di gestione (vedi §9).

## 5. Anti-obiettivi (cose che lo renderebbero un fallimento)

- **Progetto strutturato male**, che dopo mesi di lavoro si rivela inutilizzabile o da rifare. Contromisure obbligatorie:
  - ogni fase si chiude con qualcosa che si usa davvero a casa;
  - le parti rischiose si verificano prima con piccoli test usa-e-getta (spike, §11);
  - i numeri di velocità sono misurati automaticamente.
- **Rovinare i file dell'utente.** Mai modificare o spostare un file di una libreria senza conferma esplicita (l'unica eccezione è la cartella `inbox`, vedi §8.7 punto 11); prima di ogni modifica si salva lo stato precedente (tag originali e percorso originale) in un registro che permette di annullare.
- **Perdere i dati personali.** Preferiti, playlist, valutazioni e cronologia non devono mai sparire perché un file è stato spostato, ritaggato, sostituito o perché un disco non era montato durante una scansione (§8.1).
- **Setup che scoraggia.** Nessun passaggio di configurazione in più se non è indispensabile (es. MeTube funziona con il solo indirizzo, senza cartelle condivise).
- **Diventare "solo un Navidrome in Rust"**: se il riordino della libreria non è usabile, Armonika non ha motivo di esistere.

## 6. Decisioni prese con l'utente

- **Licenza: AGPL-3.0, senza CLA.**
- **Glossario** (inizio documento): file, traccia, registrazione, canzone, edizione, album e le altre parole hanno un solo significato. Dati personali legati alla traccia.
- **Multi-utente fin dall'inizio, con librerie e permessi.** Possono esistere più librerie; per ognuna l'admin decide chi accede e con quale permesso (ascolto o gestione). Una libreria condivisa e una personale sono lo stesso meccanismo. Chi ha il permesso di gestione su una libreria può aggiungere e sistemare musica in quella libreria. Playlist private di default, pubblicabili in sola lettura. Ascolti contemporanei senza limiti, ognuno per conto suo. Dettagli completi in §8.5.
- **Due API**: OpenSubsonic (compatibilità) e un'API nativa di Armonika (tutto ciò che Subsonic non sa esprimere, e base dell'interfaccia web).
- **Interfaccia web di gestione presto** (fase 2), player web dopo.
- **Modalità headless**: l'interfaccia web si può disattivare dalla configurazione; restano le API.
- **MeTube**: opzionale; di default basta l'indirizzo dell'istanza e Armonika recupera il file via rete; la cartella condivisa è solo un'opzione per chi ce l'ha già.
- **Multipiattaforma e Docker** (§7.1).
- **Ordine delle fasi**: prima le fondamenta di ascolto, poi riordino e interfaccia, poi MeTube, infine il player web.

## 7. Scelte tecniche (decise da Claude, contestabili)

### 7.1 Forma del prodotto
- **Un unico eseguibile** Rust che contiene server, entrambe le API e l'interfaccia web (i file statici dell'interfaccia sono incorporati nel binario).
- **Multipiattaforma** (requisito dell'utente). Due modi di installazione, entrambi supportati allo stesso livello:
  - **Docker**: immagine multi-architettura pubblicata su GitHub Container Registry (linux/amd64 e linux/arm64, per PC, NAS e Raspberry Pi), con un docker-compose d'esempio pronto da copiare.
  - **Binari** su GitHub Releases per Linux (x86_64 e arm64), Windows (x86_64) e macOS (Intel e Apple Silicon).
- Conseguenze sul codice: niente percorsi scritti a mano con `/` o `\` (si usano le API di Rust per i percorsi), attenzione a maiuscole/minuscole nei nomi dei file (Windows e macOS non le distinguono, Linux sì) e ai caratteri non ammessi nei nomi dei file su Windows quando Armonika rinomina i file. ffmpeg è incluso nell'immagine Docker; per i binari è una dipendenza opzionale, serve solo per la transcodifica.
- La CI compila e fa girare i test su Linux, Windows e macOS, non solo al momento del rilascio. *(Decisa con l'utente il 2026-11-12.)* Finché la repository è privata (credito CI limitato, macOS circa 10 volte più caro di Linux): Linux a ogni push su qualsiasi ramo; Windows e macOS sulle pull request verso `main` e sui push su `main`. Quando la repository diventa pubblica (CI gratuita), tutti e tre i sistemi a ogni push.
- Configurazione: file TOML + variabili d'ambiente (`ARMONIKA_*`), con default sensati; la maggior parte delle impostazioni modificabile anche dall'interfaccia.
- Cartelle (regole precise in §8.7 punto 2):
  - una cartella per ogni libreria (§8.5); dentro ognuna, Armonika usa solo la cartella nascosta `.armonika/` (cestino della libreria), che la scansione ignora;
  - una cartella `inbox` per ogni libreria, **fuori** dalla cartella della libreria (default: `inbox/<nome libreria>` accanto a `data`);
  - `data`: database, cartella di sosta della coda, cache copertine, indice di ricerca.
  - Nessuna di queste cartelle può stare dentro la cartella di una libreria, tranne `.armonika/`. Nel caso più semplice c'è una sola libreria con le cartelle `music` e `inbox`.

### 7.2 Stack backend
- Runtime async **Tokio**, server HTTP **axum**.
- Database **SQLite** in modalità WAL (un solo scrittore, molti lettori: adatto a questo uso). Migrazioni versionate fin dal primo giorno.
- Lettura/scrittura dei tag nei file con **lofty** (MP3/ID3, FLAC, Ogg Vorbis, Opus, M4A/AAC, WAV/AIFF). Sono anche i formati di file accettati.
- Transcodifica opzionale tramite **ffmpeg** esterno (presente nell'immagine Docker). Di default i file si inviano così come sono, con supporto ai range HTTP per l'avanzamento nella traccia.
- Riconoscimento: **MusicBrainz** (ricerca di registrazioni ed edizioni per metadati) e **AcoustID** (impronta audio tramite chromaprint, che porta alla registrazione) per i file con tag pessimi, come quelli da YouTube. Va rispettato il limite di MusicBrainz (1 richiesta al secondo, con User-Agent identificativo del progetto). AcoustID richiede una chiave applicazione: il progetto ne registra una propria, sostituibile dall'utente nelle impostazioni. Il riconoscimento richiede internet; senza connessione Armonika continua a funzionare e la coda accetta solo correzioni manuali.
- **Calcolo dell'impronta audio**: l'algoritmo è **Chromaprint**, creato per AcoustID dallo stesso autore. L'impronta va calcolata in locale, e deve essere identica a quella prodotta dallo strumento ufficiale `fpcalc`, altrimenti AcoustID non la riconosce. Due strade, da scegliere con lo spike 6 (§11):
  - **rusty-chromaprint**: porting in Rust puro, licenza MIT, versione 0.3 dichiarata "in progress" dall'autore. Non richiede librerie esterne;
  - **libreria C originale** (`chromaprint`): codice MIT, ma nel complesso da considerare LGPL 2.1 perché include parti di FFmpeg; compatibile con AGPL, ma complica la compilazione multipiattaforma.
  - In entrambi i casi serve prima **decodificare l'audio** di ogni formato accettato. La decodifica serve solo per l'impronta audio e per la transcodifica: lo streaming del file originale e la lettura/scrittura dei tag non ne hanno bisogno.
- **Decodifica audio** (verificato il 2026-11-12 sul README di Symphonia e sui crate pubblicati):
  - **Symphonia** (Rust puro, licenza MPL-2.0, la libreria di decodifica Rust più usata) decodifica FLAC, MP3, Vorbis, AAC-LC, ALAC, WAV/AIFF. **Non decodifica Opus** (segnato come non implementato; la richiesta è aperta dal 2020, issue #8) e nemmeno HE-AAC.
  - Opus è il formato tipico dei download da MeTube, quindi va coperto. Opzioni, da confrontare nello spike 6:
    - **libopus** (la libreria di riferimento, in C, licenza BSD) collegata a Symphonia tramite il crate `symphonia-adapter-libopus`: decoder maturo, ma aggiunge una dipendenza C da compilare per ogni piattaforma;
    - **decoder Opus in Rust puro** (es. `opus-decoder`, `opus-pure`): nessuna dipendenza C, ma progetti giovani (versioni 0.1 e 0.2 nel 2026) da validare;
    - **ffmpeg** come processo esterno: già previsto per la transcodifica, ma per i binari è una dipendenza opzionale.
  - HE-AAC: i file M4A di qualità migliore offerti da YouTube sono di solito AAC-LC, che Symphonia decodifica; per HE-AAC vale lo stesso discorso di Opus. Da misurare nello spike 6 quanti file reali ne sono interessati.
- Per ogni traccia si conservano, quando noti, gli identificativi MusicBrainz di registrazione, edizione, album, canzone e artisti.
- **Condizioni d'uso dei servizi esterni** (verificate il 2026-11-12):
  - **MusicBrainz**: i dati principali (artisti, registrazioni, edizioni, album) sono CC0, cioè di pubblico dominio; il resto è CC BY-NC-SA 3.0. L'API è gratuita per uso non commerciale, con un limite di circa 1 richiesta al secondo per indirizzo IP e l'obbligo di uno User-Agent che identifichi l'applicazione e un contatto. Chi la usa commercialmente deve accordarsi con la MetaBrainz Foundation.
  - **AcoustID**: gratuito per applicazioni non commerciali, previa registrazione dell'applicazione per ottenere una chiave; l'uso commerciale passa da piani a pagamento di AcoustID OÜ. Limite indicato dalle librerie client: 3 richieste al secondo.
  - **AcoustID non fa parte di MusicBrainz**: è un servizio separato, gestito da AcoustID OÜ. Il suo database contiene le impronte audio e, per ognuna, gli identificativi delle registrazioni MusicBrainz corrispondenti; MusicBrainz non contiene le impronte. Quindi anche chi usa un mirror locale di MusicBrainz ha bisogno del servizio online di AcoustID per il riconoscimento dal suono.
  - **Cover Art Archive** (copertine collegate a MusicBrainz): le immagini restano dei rispettivi titolari dei diritti; scaricarle per la propria libreria è l'uso comune di strumenti come Picard e beets.
- Conseguenze per Armonika:
  - ogni installazione fa le richieste dal proprio indirizzo IP, quindi il limite vale per ogni server, non per tutto il progetto;
  - User-Agent: `Armonika/<versione> ( <url del progetto> )`;
  - il progetto registra una chiave AcoustID come applicazione open source non commerciale; la chiave è sostituibile nelle impostazioni, e la documentazione deve dire chiaramente che **chi usa Armonika per un servizio commerciale deve usare chiavi e accordi propri** con AcoustID e MetaBrainz;
  - l'indirizzo del server MusicBrainz è configurabile, così chi vuole può usare un proprio mirror locale di MusicBrainz (senza limiti di richieste). È un'opzione per utenti esperti, non il caso normale: secondo il progetto ufficiale `musicbrainz-docker` servono circa 100 GB di disco, 4 GB di RAM e 2 thread senza ricerca testuale (bastano le ricerche per identificativo, cioè il percorso AcoustID), oppure 350 GB, 16 GB di RAM e 16 thread con la ricerca testuale; solo x86-64. Armonika deve funzionare bene anche senza mirror;
  - per il limite di 1 richiesta al secondo, l'analisi della coda e della revisione di tracce avviene **in background**, con avanzamento visibile e risultati salvati (non si rifà la stessa richiesta due volte). Riconoscere una libreria molto grande richiede ore, non secondi: l'interfaccia lo deve dire in modo chiaro;
  - nella documentazione pubblica e nell'interfaccia si citano MusicBrainz, AcoustID e Cover Art Archive come fonti dei dati.
- API nativa REST/JSON con specifica **OpenAPI** generata dal codice; aggiornamenti in tempo reale (scansione, download MeTube, coda) via **Server-Sent Events**.

### 7.3 Ricerca
- I dati stanno in SQLite; la ricerca è isolata dietro un'interfaccia propria (`SearchIndex`), così il motore si può sostituire senza toccare il resto.
- Si cercano tracce, album della libreria e artisti, solo nelle librerie a cui l'utente ha accesso.
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
    armonika-core/      modello di dominio (Glossario), database, migrazioni
    armonika-scanner/   scansione dei file, lettura tag, collegamento file-traccia
    armonika-search/    SearchIndex e implementazioni
    armonika-organizer/ coda, riconoscimento (MusicBrainz/AcoustID), scrittura tag, registro annullamenti
    armonika-subsonic/  API OpenSubsonic
    armonika-api/       API nativa
    armonika-metube/    client MeTube
    armonika-server/    eseguibile: config, avvio, wiring, incorpora la UI
  web/                  interfaccia web
  site/                 sito e documentazione pubblica (Astro + Starlight, più avanti)
  bench/                generatore di librerie finte e benchmark
  docs/
```

Regola: le crate API dipendono dal core, mai il contrario; l'integrazione MeTube è isolata e disattivabile. I nomi dei tipi nel codice seguono il Glossario (in inglese: `File`, `Track`, `Recording`, `Work`, `Release`, `ReleaseGroup`, `LibraryAlbum`, `Library`, `QueueItem`, `QueueGroup`). *(La suddivisione esatta può essere semplificata dal builder se all'inizio alcune crate sono troppo piccole; vincolanti sono i confini, non il numero di crate.)*

### 7.6 Interfaccia web
- **Svelte 5 + TypeScript + Vite**, compilata in file statici incorporati nel binario.
- Client dell'API generato dalla specifica OpenAPI.
- Brand: colori e logo da `images/armonika-brand-kit/BRAND.md` (sfondo scuro `#0F1115`, unico accento ruggine `#C2452D`, testo crema `#F4F1EC`). Tema scuro per primo.
- Lingue: inglese e italiano, con struttura pronta per altre traduzioni.
- Sito del progetto e documentazione pubblica (non l'interfaccia dell'app): **Astro + Starlight**, nella cartella `site/` della stessa repository. Priorità bassa, da costruire vicino al rilascio pubblico. *(Scelta di Claude, l'utente non la considera fondamentale.)*

### 7.7 Lingua del progetto
Codice, commenti, README, documentazione pubblica e messaggi di commit in **inglese**, perché il progetto è open e internazionale. I documenti interni di pianificazione possono restare in italiano.

Stile dei testi: niente segni tipici del testo generato da AI (lineette lunghe, frecce, carattere dei puntini di sospensione, virgolette tipografiche, titoli pieni di emoji). Si usa punteggiatura semplice. Vale per documentazione, commenti nel codice, commit, PR e testi dell'interfaccia.

## 8. Comportamenti chiave

### 8.1 Tracce, file e identità
- **I file sono la verità per la musica**: tag, copertine e struttura delle cartelle si possono sempre rileggere dai file, e i dati musicali di una traccia sono sempre i tag del suo file (Glossario). Il database ne conserva una copia solo per velocità (ricerca, navigazione), che la scansione riallinea ai file. **Il database è la verità per i dati personali** (utenti, preferiti, valutazioni, playlist, cronologia) e per l'identità delle tracce; questi dati sono esportabili.
- Ogni traccia ha un **identificativo stabile**, generato da Armonika alla creazione della traccia. Non dipende dal percorso del file né dai tag.
- Quando è Armonika a spostare, rinominare, ritaggare o sostituire un file, aggiorna il collegamento traccia-file nella stessa operazione: la traccia resta la stessa.
- Quando un file viene spostato o rinominato **da fuori** (es. a mano, dentro la stessa libreria), la scansione riconosce che è lo stesso file confrontando un'impronta del contenuto audio (che non cambia se cambiano solo i tag) e ricollega la traccia al nuovo percorso. *(Scelta di Claude; il tipo esatto di impronta lo decide il builder.)*
- Quando il file di una traccia **sparisce** (cancellato, oppure disco non montato), la traccia diventa **"mancante"**: non compare più nelle app, ma la traccia e tutti i dati personali collegati restano. Se il file ricompare, la traccia torna com'era. L'admin può eliminare definitivamente le tracce mancanti dall'interfaccia. *(Scelta di Claude.)*
- Un file nuovo trovato dalla scansione nella cartella di una libreria (senza passare dalla coda) diventa una **nuova traccia**, così com'è (§8.3).
- **Identità degli album della libreria** *(scelta di Claude, approvata dall'utente)*. Le tracce si raggruppano in album della libreria per edizione MusicBrainz, se nota; altrimenti per titolo dell'album e accredito dell'album nei tag. Ogni album della libreria ha un identificativo stabile, a cui sono legati i preferiti sugli album. Se i tag li cambia Armonika, l'album della libreria mantiene il suo identificativo. Se per modifiche fatte da fuori un album della libreria si divide, l'identificativo (e quindi i preferiti) resta alla parte con più tracce; se due si uniscono, resta quello con più tracce e i preferiti dell'altro si spostano su di esso.
- **Identità degli artisti** *(scelta di Claude, approvata dall'utente)*. Un artista è identificato dall'identificativo MusicBrainz se noto, altrimenti dal nome normalizzato (maiuscole, spazi e accenti uniformati). Gli accrediti con più artisti vengono separati negli artisti che li compongono solo in modo sicuro: con gli identificativi MusicBrainz se presenti, altrimenti con i tag multi-valore degli artisti (es. `ARTISTS`, scritto da Picard e da Armonika), altrimenti solo sui separatori non ambigui (`feat.`, `ft.`, `featuring`, `;`). **Non si divide mai su `&`, `and`, `e` o sulla virgola**, così nomi come "Simon & Garfunkel" o "Earth, Wind & Fire" restano un solo artista. Un accredito non divisibile in modo sicuro resta un solo artista; la revisione di tracce può correggerlo scrivendo gli identificativi. I preferiti sugli artisti sono legati all'artista. Regole di identità complete in §8.7 punto 7.

### 8.2 La coda "da sistemare" e il riconoscimento
- In coda finiscono gli elementi in coda: i file caricati dal browser, i file nella cartella `inbox`, i file scaricati da MeTube. Le vie di ingresso sono descritte in §8.5, "Come entra la musica"; il percorso completo in §8.6.
- Separata dalla coda esiste la **revisione di tracce** (Glossario): chi ha la gestione può chiedere di rivedere tracce già presenti con problemi. Usa lo stesso riconoscimento e lo stesso schermo, ma lavora su tracce e si può usare solo se la scrittura sulla libreria è attiva (§8.3).
- Per ogni gruppo (e per ogni revisione di tracce) Armonika propone una o più corrispondenze: per ogni file, la registrazione; per il gruppo, l'edizione. Mostra i tag proposti e la copertina, con le differenze rispetto ai tag attuali.
- **Il grado di confidenza lo calcola Armonika.** MusicBrainz e AcoustID restituiscono dei punteggi, ma non misurano "quanto è probabile che sia la corrispondenza giusta":
  - lo `score` di AcoustID (da 0 a 1) misura solo quanto l'impronta audio del file somiglia a una registrazione nota; non dice in quale edizione si trova;
  - lo `score` della ricerca di MusicBrainz (da 0 a 100) misura la somiglianza del testo cercato. Verifica fatta il 2026-11-12: cercando "Bohemian Rhapsody" dei Queen, i primi 10 risultati hanno tutti score 100, tra cui il mix 5.1 del 2001 e due registrazioni live.
  - Armonika combina quindi più indizi in un proprio punteggio da 0 a 100: punteggio AcoustID, differenza di durata tra file e registrazione, somiglianza di titolo e artista con i tag attuali e, per un gruppo, quanti file corrispondono alle tracce dell'edizione e se il numero coincide. A parità di registrazione, preferisce l'edizione originale dell'album rispetto a raccolte, salvo indizi contrari (es. tutti i file del gruppo corrispondono a una raccolta).
  - I pesi dei singoli indizi e le soglie (es. sotto quale valore segnalare "corrispondenza incerta") si stabiliscono con lo spike 4 (§11), misurando i risultati su file veri.
- Ogni libreria ha la sua coda. La vede e la gestisce solo chi ha il permesso di gestione su quella libreria.
- Chi ha il permesso di gestione conferma, corregge o scarta. Cosa succede alla conferma è descritto in §8.6.
- **Registro e annullamento**: ogni operazione su file e tracce viene registrata con lo stato precedente (tag e percorso) e si può annullare.
- Applicazione automatica sopra una certa confidenza: **non nella prima versione**; il design la deve rendere possibile in seguito.

### 8.3 Scrittura sui file già presenti in una libreria
- Per default Armonika **non scrive** nei file delle tracce già presenti in una libreria (livello "Sola lettura"; la cartella può essere montata in sola lettura). Gli altri livelli sono "Solo tag" e "Tag e organizzazione" (§8.7 punto 4). Scrivere tag o spostare quei file è un'opzione che l'admin attiva esplicitamente, libreria per libreria (livelli esatti in §8.7 punto 4). Finché la scrittura non è attiva, le tracce di quella libreria non si possono correggere né rivedere (Glossario: i dati musicali di una traccia sono i tag del suo file).
- I file che arrivano dalla coda (browser, MeTube, `inbox`) vengono invece sempre scritti e spostati nella libreria alla conferma, perché è lo scopo dell'importazione. Questo richiede che la cartella della libreria sia scrivibile; se non lo è, Armonika lo segnala e la coda di quella libreria non accetta conferme.

### 8.4 MeTube
- Impostazione (solo admin): indirizzo dell'istanza MeTube (ed eventuali credenziali). Senza, la funzione non appare.
- "Aggiungi da link" è disponibile solo a chi ha il permesso di gestione su almeno una libreria. Si sceglie la libreria di destinazione tra quelle in cui si ha il permesso.
- Armonika invia la richiesta a MeTube (formato audio) e mostra lo stato del download (in coda, in corso, completato, fallito). Al completamento recupera il file via HTTP da MeTube, e il file diventa un elemento in coda nella libreria scelta.
- Opzionale: se la cartella dei download di MeTube è montata anche in Armonika, il file si legge da lì senza scaricarlo.
- Formato richiesto a MeTube: audio nella migliore qualità disponibile, senza riconversioni inutili (di solito Opus o M4A), con la copertina incorporata se MeTube lo consente. Scelta modificabile nelle impostazioni. *(Scelta di Claude.)*
- Il formato delle risposte di MeTube non è uno standard stabile: il client va isolato e coperto da test contro una vera istanza MeTube (in Docker).

### 8.5 Utenti, librerie e permessi

Questa sezione definisce in modo preciso cosa significa "multi-utente". Le scelte marcate *(decisa con l'utente)* non si cambiano senza tornare da lui; le altre sono scelte di Claude.

**Ruoli dell'account**
- **Admin del server**: crea e gestisce utenti e librerie, assegna i permessi, gestisce le impostazioni del server e il collegamento a MeTube. Ha automaticamente il permesso di gestione su tutte le librerie. Possono esserci più admin; non si può eliminare o declassare l'ultimo admin.
- **Utente**: accede solo alle librerie per cui ha un permesso. Non vede le impostazioni del server.

**Permesso "download offline"** *(decisa con l'utente)*
- È un permesso dell'account, non della libreria: l'admin lo attiva o disattiva per ogni utente. È attivo di default.
- Permette di salvare tracce sul proprio dispositivo per ascoltarle offline (funzione delle app Subsonic, permesso `downloadRole` dell'API Subsonic) e di scaricare il file di una traccia dall'interfaccia web.
- Vale solo per le librerie a cui l'utente ha accesso. Senza questo permesso l'utente può solo ascoltare in streaming.
- Da non confondere con "aggiungere musica" al server, che dipende dal permesso di gestione.

**Nuovo utente** *(decisa con l'utente)*
- Un utente appena creato **non ha accesso a nessuna libreria**. Al momento della creazione l'admin sceglie le librerie e il permesso per ognuna (ascolto o gestione), e se attivare il download offline.

**Librerie** *(decisa con l'utente)*
- Una libreria è una cartella di musica sul server, con un nome. Possono essercene quante se ne vuole.
- Le cartelle di due librerie **non possono sovrapporsi** (nessuna libreria dentro un'altra), così ogni file appartiene a una sola libreria e ogni traccia esiste in una sola libreria.
- La stessa edizione presente in due librerie è fatta di file diversi, quindi di tracce diverse e di due album della libreria distinti. Non si cerca di unirli e non si segnala nulla.
- "Libreria condivisa" e "libreria personale" non sono tipi diversi: sono librerie con più utenti o con un solo utente ammesso.

**Permessi per libreria** *(decisa con l'utente)*
- **Ascolto**: vedere, cercare e ascoltare le tracce della libreria.
- **Gestione**: ascolto, più aggiungere musica (caricamento dal browser, MeTube), usare la coda, modificare i tag delle tracce, rinominare e spostare i loro file dentro la libreria (se la scrittura è attiva, §8.3), eliminare tracce, annullare le operazioni del registro.
- Spostare tracce da una libreria a un'altra richiede il permesso di gestione su entrambe. Lo spostamento sposta il file; la traccia mantiene il suo identificativo e i dati personali collegati. *(Scelta di Claude.)*
- Senza permesso, una libreria è invisibile: le sue tracce non compaiono nella ricerca, nella navigazione, nelle API né nei conteggi.

**Cestino ed eliminazione** *(scelta di Claude)*
- Il cestino di una libreria sta nella cartella nascosta `.armonika/cestino/` dentro la cartella della libreria (stesso disco, quindi spostamenti istantanei); quello degli elementi in coda scartati sta in `data`. Eliminare tracce o sostituire file richiede che Armonika possa modificare i file esistenti (§8.7 punto 4).
- Il cestino esiste solo nelle librerie al livello "Tag e organizzazione" (§8.7 punto 4).
- Eliminare una traccia sposta il suo file nel cestino della libreria. La traccia risulta "eliminata": sparisce dalle app e dalle playlist, ma i dati personali collegati restano finché il cestino non viene svuotato.
- Ripristinare dal cestino riporta il file al suo posto e la traccia ricompare, anche nelle playlist, con tutti i dati personali.
- Svuotare il cestino (chi può farlo: vedi §8.7 punto 12) cancella definitivamente i file e le tracce eliminate, con i dati personali collegati; le tracce spariscono definitivamente dalle playlist.
- Gli elementi in coda scartati e i file sostituiti (§8.6) finiscono anch'essi nel cestino, come file senza traccia.

**Come entra la musica in una libreria** *(decisa con l'utente)*

Riassunto: **aggiunge musica a una libreria solo chi ha il permesso di gestione su quella libreria**. Chi ha solo l'ascolto non può aggiungere nulla, in nessun modo. Le vie di ingresso sono tre, e tutte producono elementi in coda nella libreria scelta: nessun file arriva direttamente in libreria come traccia.

| Via | Chi la può usare | Note |
|---|---|---|
| Caricamento dal browser | chi ha la gestione sulla libreria di destinazione | Si trascinano file audio o un archivio zip nell'interfaccia web. Dimensione massima per caricamento configurabile dall'admin (default: 2 GB). Formati di file accettati: quelli del §7.2; gli altri file vengono gestiti come in §8.6. *(Default e zip: scelte di Claude.)* |
| MeTube | chi ha la gestione sulla libreria di destinazione | Vedi §8.4. Disponibile solo se l'admin ha collegato MeTube. |
| Cartella `inbox` della libreria | chi ha accesso ai file del server (in pratica l'admin) | Si copiano i file nella cartella, per esempio via rete o via SSH. Armonika li rileva da solo. |

- Ogni elemento in coda ricorda **chi l'ha aggiunto e da dove** (browser, MeTube, inbox), ed è visibile a tutti quelli che hanno la gestione su quella libreria.
- I file già presenti nella cartella della libreria, o copiati lì a mano, non passano dalla coda: la scansione li trasforma direttamente in tracce, con i tag che hanno (§8.1).

**Sicurezza dei caricamenti** (requisito, con test dedicati)
- Un file caricato o scaricato da MeTube non deve mai poter finire fuori dalla cartella di sosta della sua coda. I nomi dei file vengono ripuliti e i percorsi verificati dopo la normalizzazione.
- Gli archivi zip vengono estratti con lo stesso controllo: le voci con percorsi come `../`, percorsi assoluti o collegamenti simbolici vengono scartate (attacco noto come "zip slip"). Si controlla anche la dimensione totale estratta, per evitare archivi che si espandono a dismisura.
- Motivo: in Airsonic-Advanced è stata pubblicata nel 2025 una vulnerabilità in cui un utente autenticato, tramite il caricamento delle playlist, poteva scrivere file fuori dalla cartella prevista.

**Limiti di spazio** *(decisa con l'utente)*
- Lo spazio si misura sempre sulla dimensione dei file.
- **Spazio massimo per libreria**: quanto possono occupare in totale i file della libreria, inclusi quelli in sosta nella sua coda e nel suo cestino.
- **Spazio massimo per utente**: la somma dei file che quell'utente ha aggiunto (tramite la coda), in tutte le librerie, inclusi quelli in sosta. Ogni file conta per chi l'ha aggiunto; se un file viene sostituito, il nuovo conta per chi ha aggiunto il nuovo.
- Entrambi impostabili dall'admin, **illimitati di default**.
- Il controllo avviene prima di accettare un caricamento o un download MeTube (in base alla dimensione dichiarata) e di nuovo alla fine (in base alla dimensione reale). Se il limite verrebbe superato, l'operazione viene rifiutata con un messaggio chiaro che indica spazio usato e spazio disponibile. Le tracce già presenti restano sempre ascoltabili.
- Se l'admin abbassa un limite sotto lo spazio già usato, non si cancella nulla: si bloccano solo i nuovi caricamenti.
- I file presenti nella libreria prima di Armonika, o copiati a mano nella cartella, contano per lo spazio della libreria ma non per quello di nessun utente.

**Esempio concreto (famiglia)**
- Libreria "Famiglia": l'admin ha la gestione, gli altri l'ascolto. Solo l'admin aggiunge musica; tutti ascoltano e, se hanno il download offline, salvano le tracce sul telefono.
- Libreria "Marco": Marco ha la gestione ed è l'unico ammesso. Solo lui la vede e ci aggiunge musica (dal browser o da MeTube). L'admin può comunque gestirla, perché è admin.

**Dati personali di ogni utente**
- Preferiti (su tracce, album della libreria e artisti), valutazioni, numero di ascolti, cronologia, playlist, coda di riproduzione salvata, collegamenti a Last.fm e ListenBrainz, preferenze (es. qualità della transcodifica), credenziali per le app Subsonic.
- Sono sempre per utente e, per quanto riguarda la musica, legati alla traccia (Glossario): il "numero di ascolti" di una traccia è il tuo, non quello di tutta la famiglia, e due tracce con la stessa registrazione hanno conteggi separati.

**Playlist** *(decisa con l'utente)*
- Una playlist è un elenco ordinato di tracce. La stessa traccia può comparire più volte, e tracce diverse con la stessa registrazione o la stessa canzone possono stare nella stessa playlist.
- Private di default. Il proprietario può renderle **pubbliche**: gli altri utenti le vedono e le ascoltano, ma non le modificano.
- Una playlist può contenere tracce di più librerie. Chi la guarda vede solo le tracce delle librerie a cui ha accesso; le altre sono nascoste, non mostrate come errori. Lo stesso vale per tracce mancanti o eliminate (§8.1, cestino).

**Ascolto contemporaneo** *(decisa con l'utente)*
- Nessun limite al numero di utenti e dispositivi che ascoltano insieme; ognuno ascolta la sua musica in modo indipendente.
- Lo stesso utente può usare più dispositivi. La coda di riproduzione salvata permette di riprendere su un altro dispositivo (funzione Subsonic `savePlayQueue`/`getPlayQueue`); se due dispositivi la salvano, vale l'ultimo.
- "Cosa stanno ascoltando gli altri" (funzione Subsonic `getNowPlaying`): per privacy ogni utente vede solo il proprio ascolto, a meno che non scelga di renderlo visibile agli altri.
- L'ascolto di gruppo non si costruisce ora; il design non deve impedirlo in futuro (es. non legare la coda di riproduzione a un singolo dispositivo in modo rigido).

**Cambi di permessi e cancellazioni**
- Se un utente perde l'accesso a una libreria, i suoi preferiti, valutazioni e cronologia sulle tracce di quella libreria restano salvati ma diventano invisibili; tornano se l'accesso viene ridato.
- Se si elimina un utente, si cancellano i suoi dati personali e le sue playlist (anche quelle pubbliche). Tracce e file non vengono mai toccati.
- Se si elimina una libreria da Armonika, si rimuovono dal database le sue tracce e i dati collegati, ma **non i file** sul disco, compreso il cestino in `.armonika/`. Gli elementi in coda di quella libreria (in `data`) vengono cancellati. Prima di procedere l'interfaccia mostra quante tracce, playlist ed elementi in coda sono coinvolti e chiede conferma. *(Scelta di Claude.)*

**Come si vede dalle app Subsonic**
- Ogni libreria corrisponde a una "cartella musicale" Subsonic (`getMusicFolders`). Ogni utente vede solo le proprie, e le app che lo supportano permettono di filtrare per libreria.
- Una "song" Subsonic è una traccia; un "album" Subsonic è un album della libreria. Gli identificativi esposti sono quelli stabili delle tracce, così le app non perdono nulla quando un file viene spostato o sostituito.

**Cosa c'è in ogni fase**
- Fase 1: il modello completo (Glossario, librerie, permessi, dati personali legati alle tracce, tracce mancanti) esiste già nel database e nelle API Subsonic. Librerie e permessi si gestiscono dal file di configurazione o con comandi dell'eseguibile.
- Fase 1: il permesso "download offline" è già rispettato dall'API Subsonic.
- Fase 2: gestione di utenti, librerie e permessi dall'interfaccia web; caricamento dal browser; limiti di spazio; cestino.

### 8.6 Percorso di un file importato

Questa sezione descrive passo per passo cosa succede a un file dall'arrivo fino a quando diventa una traccia ascoltabile. Vale per tutte le vie di ingresso (browser, MeTube, `inbox`). Integra §8.2 e §8.5; in caso di dubbio vale questa sezione.

**1. Arrivo**
- Possono aggiungere musica solo gli utenti con permesso di gestione sulla libreria di destinazione (§8.5). Valgono i limiti di spazio e i controlli di sicurezza (§8.5).
- **Caricamento a pezzi con ripresa.** Il caricamento dal browser divide i file in parti; se la connessione cade, riprende da dove si era fermato invece di ricominciare. Va previsto nell'API nativa dalla prima versione del caricamento. *(Scelta di Claude.)*
- **Le app Subsonic non possono caricare**: l'API Subsonic non lo prevede. Si carica dall'interfaccia web o dall'API nativa (quindi anche da script e strumenti di terzi).
- **File non audio** (anche dentro uno zip): si tengono le immagini di copertina (`.jpg`, `.jpeg`, `.png`, `.webp`) e i testi sincronizzati (`.lrc`), associati ai file audio della stessa cartella. Tutto il resto (`.cue`, `.log`, `.pdf`, `.txt`, eccetera) viene scartato con un avviso. *(Scelta di Claude.)*
- Ogni file audio accettato diventa un **elemento in coda**.

**2. Sosta in coda**
- I file degli elementi in coda stanno in una **cartella di sosta** dentro `data`, separata da ogni libreria. Non sono tracce: non compaiono in ricerca, navigazione e API Subsonic. Chi ha la gestione sulla libreria può ascoltarli in anteprima dall'interfaccia web.
- Nessuna scadenza automatica: un elemento resta in coda finché qualcuno decide. L'interfaccia mostra da quanto tempo aspetta.
- Lo spazio dei file in sosta conta già nei limiti di spazio (libreria e utente).
- Scartare un elemento sposta il suo file nel cestino della libreria (§8.5), non lo cancella subito.

**3. Analisi**
- Armonika legge i tag di ogni file, ne calcola l'impronta audio, cerca registrazione ed edizione corrispondenti e calcola il proprio punteggio di confidenza (§8.2).
- **Unità di revisione: il gruppo** *(decisa con l'utente)*. I gruppi si formano così (regola precisa in §8.7 punto 15): prima per stessa cartella di provenienza **e** stessi tag di titolo dell'album e artista dell'album; dopo il riconoscimento, Armonika propone di unire i gruppi riconosciuti come la stessa edizione. Si confermano insieme, come fa beets. Un elemento isolato (es. da MeTube) è un gruppo da uno. Chi gestisce può separare un elemento da un gruppo o unire due gruppi.
- **Controllo doppioni** *(regola decisa con l'utente)*. Un elemento in coda è un doppione se ha **la stessa registrazione e la stessa edizione** di una traccia già presente **nella stessa libreria**.
  - Esempi di doppione: la stessa edizione caricata due volte; la stessa edizione prima in MP3 e poi in FLAC.
  - **Non sono doppioni**: la stessa registrazione in un'altra edizione (es. una raccolta "Greatest Hits" che contiene la registrazione dell'album originale); registrazioni diverse della stessa canzone (studio, live, remix, cover); qualsiasi traccia in un'altra libreria. In questi casi **non si segnala nulla**.
  - Se registrazione o edizione non sono state riconosciute, l'elemento si considera doppione solo se l'impronta audio è quasi identica a quella di una traccia della stessa libreria **e** nei tag coincidono titolo dell'album e numero di traccia. Altrimenti non si segnala nulla. *(Scelta di Claude.)*
  - Per un doppione l'avviso si mostra sempre, chiunque abbia aggiunto la traccia già presente, con chi l'ha aggiunta, quando e il confronto di qualità dei due file (formato, bitrate, durata). Chi gestisce sceglie, senza opzione predefinita:
    - **sostituisci** (disponibile solo nelle librerie al livello "Tag e organizzazione", §8.7 punto 4, perché tocca un file esistente): il file nuovo diventa il file della traccia esistente; il vecchio file va nel cestino. La traccia resta la stessa, quindi preferiti, valutazioni, conteggi e playlist non cambiano;
    - **tieni entrambi**: l'elemento diventa una nuova traccia, distinta da quella esistente;
    - **scarta**: il file dell'elemento va nel cestino.

**4. Revisione e conferma**
- **Ogni gruppo passa da una conferma umana** *(decisa con l'utente)*, anche se i tag sono già perfetti. Per rendere veloce il caso facile ci sono "conferma così com'è" (tiene i tag attuali dei file) e "conferma tutti quelli sopra la soglia" (conferma in blocco i gruppi con confidenza alta). L'importazione automatica senza conferma potrà essere aggiunta dopo, quando il punteggio sarà stato misurato su file veri.
- Un elemento non riconosciuto si conferma con tag scritti a mano. Campi obbligatori: titolo e artista della traccia; se mancano l'album o il numero di traccia, il file va nella cartella dei singoli dell'artista (vedi punto 5).
- Quando una persona apre un gruppo per rivederlo, il gruppo risulta "in revisione da" quella persona, e gli altri non possono confermarlo nello stesso momento. Il blocco scade dopo 30 minuti di inattività. *(Scelta di Claude.)*
- Alla conferma, ogni elemento del gruppo diventa una **nuova traccia** (oppure sostituisce il file di una traccia esistente, se si è scelto "sostituisci").

**5. Scrittura nella libreria**
- **Formato audio: questione aperta** *(decisa con l'utente)*. La prima versione **non converte mai** i file: entrano con formato e qualità originali. È la scelta che lascia aperte entrambe le strade, perché una conversione opzionale si può aggiungere dopo, mentre un file convertito non torna alla qualità originale. La conversione per lo streaming avviene al volo e non tocca il file.
- **Copertina**: incorporata in ogni file e salvata anche come `cover.jpg` nella cartella dell'album della libreria, così la leggono tutti i programmi. *(Scelta di Claude.)*
- **Schema dei nomi** (default, configurabile per libreria): `Artista dell'album/Album (Anno)/NN - Titolo.ext`. Casi speciali:
  - raccolte di artisti vari: `Various Artists/Album (Anno)/NN - Artista della traccia - Titolo.ext`;
  - edizioni su più dischi: `Album (Anno)/Disco N/NN - Titolo.ext`;
  - due edizioni dello stesso album nella stessa libreria: alla seconda si aggiunge la distinzione nel nome della cartella (es. `Album (Anno) [Remaster 2011]`);
  - file senza titolo dell'album nei tag: `Artista della traccia/Singoli/Titolo.ext`;
  - caratteri non ammessi nei nomi di file su Windows, macOS o Linux vengono sostituiti; nomi troppo lunghi vengono accorciati;
  - se due file finirebbero nello stesso percorso, il secondo prende un suffisso (`(2)`) e la cosa viene segnalata. **Non si sovrascrive mai un file.**
- **Operazione a prova di interruzione.** Ogni conferma è un'operazione registrata nel database a passi: copia del file nella libreria con un nome temporaneo, scrittura dei tag sulla copia, verifica che la copia sia leggibile, rinomina al nome definitivo, creazione o aggiornamento della traccia, indicizzazione, rimozione del file dalla sosta. Se il server si spegne a metà, al riavvio Armonika completa o annulla l'operazione; un file non resta mai a metà e non si perde mai l'originale prima che la copia sia verificata.

**6. Dopo l'importazione**
- Ogni file ricorda chi l'ha aggiunto, quando e da quale via; questo dato serve ai limiti di spazio e alla cronologia, non dà diritti speciali.
- **In una libreria condivisa non esistono proprietari delle tracce**: chiunque abbia la gestione su quella libreria può modificare o eliminare qualsiasi traccia, anche aggiunta da altri. Chi vuole musica che controlla solo lui la mette nella propria libreria personale.
- Se un utente perde il permesso di gestione o viene eliminato, le tracce che ha aggiunto restano nella libreria.
- L'operazione resta nel registro e si può annullare (§8.2).

### 8.7 Regole precise per i casi limite

Questa sezione nasce dalla revisione del documento fatta da un lettore esterno il 2026-11-12. Ogni punto chiude un caso che altrimenti il builder dovrebbe indovinare. Salvo dove indicato, sono scelte di Claude.

**1. Quando un file trovato dalla scansione si ricollega a una traccia esistente**
- Si ricollega automaticamente solo se l'**impronta del contenuto** (§8.1) coincide con quella di una traccia mancante **della stessa libreria**, e c'è una sola traccia candidata. Se le candidate sono più di una (copie identiche), si preferisce quella con lo stesso nome di file; se resta ambiguo, non si ricollega nulla.
- Se un file viene sostituito a mano con un file diverso (es. un MP3 rimpiazzato da un FLAC), l'impronta cambia: nasce una nuova traccia e la vecchia diventa mancante. Armonika lo rileva (stessa cartella, stessi tag principali o stessa registrazione ed edizione) e **propone** a chi ha la gestione di ricollegare: la nuova traccia prende il posto della vecchia, con i suoi dati personali. Non lo fa mai da solo.
- Un file spostato a mano in **un'altra libreria** diventa una nuova traccia in quella libreria; quella vecchia diventa mancante. Vale la stessa proposta di ricollegamento, riservata a chi ha la gestione su entrambe le librerie.

**2. Cartelle su disco**
- Cestino della libreria: `.armonika/cestino/` dentro la cartella della libreria. La scansione ignora tutta la cartella `.armonika/`.
- `inbox`: sempre fuori dalla cartella della libreria (default `inbox/<nome libreria>` accanto a `data`), configurabile.
- Cartella di sosta della coda e cestino degli elementi in coda scartati: dentro `data`.
- Se la libreria è in sola lettura, `.armonika/` non esiste e le operazioni che la richiedono (eliminare, sostituire) non sono disponibili.

**3. Annullare (registro)**
- Si può annullare **qualsiasi singola operazione**, non solo l'ultima, a una condizione: lo stato attuale dei file coinvolti deve essere ancora quello lasciato dall'operazione (Armonika lo verifica con l'impronta del contenuto e i tag). Se nel frattempo il file è cambiato, l'annullamento è bloccato e l'interfaccia spiega perché.
- Annullare una **modifica di tag o una rinomina**: si riscrivono i tag precedenti e si riporta il file al percorso precedente.
- Annullare una **conferma di importazione**: il file torna nella coda come elemento in coda; la traccia creata diventa "eliminata" (come nel cestino), con i dati personali raccolti nel frattempo, che tornano se lo stesso file viene riconfermato come quella traccia.
- Annullare una **sostituzione**: il vecchio file torna a essere il file della traccia e quello nuovo va nella coda; possibile solo finché il vecchio file è nel cestino.
- Il registro si conserva per sempre; l'annullamento è possibile finché le condizioni sopra sono vere. Lo stato precedente salva tutti i tag testuali; le copertine incorporate si salvano una sola volta per contenuto (stessa immagine, una sola copia), per contenere lo spazio.

**4. Cosa può fare Armonika sui file esistenti di una libreria** *(decisa con l'utente)*

Per ogni libreria l'admin sceglie uno di tre livelli. Vale per i file delle tracce già presenti; i file importati dalla coda vengono sempre scritti e sistemati secondo lo schema dei nomi (§8.3).

| Livello | Tag dei file esistenti | Rinomina e spostamento secondo lo schema | Cestino, eliminazione, sostituzione, unione di doppioni | Revisione di tracce |
|---|---|---|---|---|
| **Sola lettura** (default) | no | no | no | no |
| **Solo tag** | sì | no: i file restano nelle cartelle e con i nomi scelti dall'utente | no | sì, ma alla conferma si scrivono solo i tag |
| **Tag e organizzazione** | sì | sì | sì | sì, completa |

- Con "Solo tag" e "Sola lettura" la cartella nascosta `.armonika/` non serve e non viene creata.
- Passare a un livello più alto non modifica nulla da solo: i file cambiano solo con revisioni confermate. In particolare, attivare "Tag e organizzazione" **non** riorganizza automaticamente tutta la libreria; per farlo serve un'azione esplicita "riorganizza secondo lo schema", con anteprima di tutti gli spostamenti e conferma. *(Scelta di Claude.)*

**5. Doppioni: come si rileva l'audio quasi identico**
- Per gli elementi in coda si usa l'impronta Chromaprint (quella di AcoustID), già calcolata per il riconoscimento.
- Per le tracce già in libreria l'impronta Chromaprint **non** si calcola durante la scansione (rallenterebbe troppo, §3). Si calcola in background, a bassa priorità, dopo la scansione, e si salva. Finché non c'è, il confronto si fa solo sulle tracce candidate (stessi tag di album, numero di traccia e durata simile), calcolando l'impronta solo per quelle.
- L'impronta del contenuto (§8.1) è un'altra cosa: serve solo a riconoscere lo stesso file spostato, e si calcola senza decodificare l'audio (sui dati audio compressi, esclusi i tag).

**6. Doppioni tra tracce già presenti**
- La revisione di tracce può segnalare due tracce della stessa libreria con stessa registrazione e stessa edizione. Le scelte sono: **unisci** (si tiene un file, scelto da chi gestisce; l'altro va nel cestino), **tieni entrambe**, **ignora**.
- Unire: resta l'identificativo della traccia di cui si tiene il file. I dati personali si fondono: preferito se lo era almeno una delle due, valutazione più alta, ascolti e cronologia sommati, nelle playlist la traccia rimossa viene sostituita da quella tenuta.

**7. Identità degli artisti**
- Ogni artista ha un identificativo stabile generato da Armonika; l'identificativo MusicBrainz è un'informazione dell'artista, non la sua identità. Quando Armonika scrive un identificativo MusicBrainz, l'artista resta lo stesso.
- Un artista con identificativo MusicBrainz su alcune tracce e solo il nome su altre è **un solo artista**, se esiste un solo artista MusicBrainz con quel nome normalizzato nella libreria. Se ce ne sono più di uno (omonimi), le tracce senza identificativo restano su un artista separato finché una revisione non le collega.

**8. Identità delle librerie**
- Ogni libreria ha un identificativo stabile; nome e percorso sono proprietà modificabili.
- Il file di configurazione crea le librerie solo al primo avvio (o quelle che non esistono ancora). Da lì in poi la verità è il database, modificabile da comando o dall'interfaccia; se configurazione e database non coincidono, vince il database e Armonika lo segnala nei log.
- Cambiare il percorso di una libreria mantiene il suo identificativo; la scansione ricollega le tracce per percorso relativo e, se serve, per impronta del contenuto.

**9. Playlist con voci nascoste**
- Le voci nascoste (librerie senza accesso, tracce mancanti o eliminate) restano salvate nella playlist, nella loro posizione.
- Nelle API (Subsonic e nativa) gli indici si riferiscono sempre alla lista **visibile** a chi chiama. Una modifica, anche la sostituzione completa della playlist, non tocca mai le voci nascoste a chi la fa: restano dove sono rispetto alle voci visibili vicine.

**10. Album della libreria nei casi limite**
- Una traccia senza titolo dell'album nei tag appartiene all'album della libreria "[Singoli]" del suo artista dell'accredito principale, uno per artista e per libreria. Così le app Subsonic trovano sempre un album.
- Tracce della stessa cartella con stesso titolo dell'album e stesso artista dell'album appartengono allo stesso album della libreria anche se solo alcune hanno l'identificativo dell'edizione; la revisione di tracce segnala l'incoerenza.

**11. La cartella `inbox`**
- `inbox` è una cartella di consegna: mettere un file lì significa consegnarlo ad Armonika. Armonika lo **sposta** nella cartella di sosta (non lo copia, per non ritrovarlo due volte). È l'unica cartella in cui Armonika sposta file senza una conferma per ogni file, ed è scritto chiaramente nella documentazione e accanto all'impostazione.
- Armonika aspetta che il file sia completo (dimensione stabile per alcuni secondi) prima di prenderlo.

**12. Chi svuota il cestino** *(decisa con l'utente)*
- Chi ha la gestione su una libreria può svuotarne il cestino (tutto o singoli file); l'admin può farlo su tutte le librerie.
- L'admin può impostare per ogni libreria uno svuotamento automatico dopo N giorni; **spento di default**.
- Svuotare chiede sempre conferma e mostra quante tracce eliminate e quanti dati personali collegati verranno cancellati definitivamente.

**13. Eliminazione di una libreria**: vedi §8.5, "Cambi di permessi e cancellazioni".

**14. "Conferma così com'è" e "conferma tutti quelli sopra la soglia"**
- "Conferma così com'è": non cambia i tag del file, ma lo sposta e lo rinomina secondo lo schema dei nomi. Se mancano titolo o artista (obbligatori), la conferma è bloccata e l'elemento resta in coda con l'indicazione di cosa manca.
- "Conferma tutti quelli sopra la soglia": per ogni gruppo sopra la soglia scrive i tag e gli identificativi MusicBrainz proposti, poi sposta e rinomina. I gruppi con doppioni o con campi obbligatori mancanti vengono esclusi e restano in coda.

**15. Formazione dei gruppi**
- Prima del riconoscimento: stesso gruppo se **stessa cartella di provenienza e** stessi tag di titolo dell'album e artista dell'album. Un file senza titolo dell'album è un gruppo da solo.
- Dopo il riconoscimento: Armonika propone di unire i gruppi riconosciuti come la stessa edizione, e di separare un gruppo i cui file risultano appartenere a edizioni diverse. Chi gestisce accetta o rifiuta la proposta.

**16. Separazione degli accrediti**: vedi §8.1, "Identità degli artisti" (mai su `&`, `and`, `e` o virgola senza dati sicuri).

## 9. Fasi

Ogni fase si chiude solo quando il suo **criterio di uscita** è vero.

**Fase 0: Spike e impalcatura.** Gli spike del §11, la struttura della repository, CI (build, test, lint, benchmark), generatore di librerie finte.
*Uscita:* motore di ricerca scelto con dati alla mano; recupero dei file da MeTube verificato; CI verde.

**Fase 1: Fondamenta di ascolto.** Scansione (completa, incrementale, osservazione delle cartelle, collegamento file-traccia, tracce mancanti), database, ricerca, streaming con range HTTP, copertine, utenti, ruoli, librerie e permessi (§8.5), API OpenSubsonic (navigazione, ricerca, streaming, preferiti, playlist, scrobbling Last.fm/ListenBrainz), modalità headless, Docker.
*Uscita:* l'utente usa Armonika ogni giorno sul suo homelab con almeno due app Subsonic (es. Symfonium e Feishin), i benchmark rispettano le soglie del §3, e spostare o rinominare a mano un file non fa perdere preferiti né playlist.

**Fase 2: Il cuore di Armonika.** API nativa, interfaccia web di gestione (accesso, utenti, librerie e permessi, impostazioni, stato della scansione, coda, caricamento dal browser, revisione dei gruppi, revisione di tracce, modifica dei tag, registro e annullamento, cestino), riconoscimento con MusicBrainz + AcoustID e punteggio di confidenza calcolato da Armonika, controllo doppioni, scrittura dei tag, rinomina e spostamento dei file, limiti di spazio.
*Uscita:* partendo dalla libreria "disordinata apposta" (§10), l'utente la mette in ordine interamente dal browser, e ogni operazione si può annullare.

**Fase 3: MeTube.** Collegamento, "Aggiungi da link", stato dei download, recupero del file, ingresso in coda.
*Uscita:* dal divano, si incolla un link in Armonika e, dopo la conferma, la nuova traccia è ascoltabile da un'app Subsonic, senza aprire MeTube.

**Fase 4: Player web.** Riproduzione nel browser dentro la stessa interfaccia.

La repository resta **privata** durante lo sviluppo. Il rilascio pubblico (apertura della repository e prima release) è previsto indicativamente alla fine della fase 2, quando la caratteristica distintiva esiste; il momento lo decide l'utente. Siccome all'apertura diventa pubblica tutta la cronologia git, nessun segreto (chiavi, password, file personali) deve mai essere committato, nemmeno temporaneamente.

Nei commit, nelle PR e nei file del progetto non va indicato Claude come autore o co-autore.

## 10. Dati di prova

L'utente non ha ancora una libreria musicale, quindi:
- **Libreria grande finta**: generatore che crea centinaia di migliaia di file audio minuscoli (pochi ms di silenzio) con tag realistici e vari. Serve per i benchmark.
- **Libreria "disordinata apposta"**: file con licenza Creative Commons (es. Free Music Archive, Jamendo) con tag rovinati di proposito: titoli stile YouTube, tag mancanti, album della libreria spezzati, doppioni (stessa registrazione e stessa edizione in due formati), falsi doppioni (stessa registrazione in una raccolta, registrazioni diverse della stessa canzone), copertine assenti, formati diversi. Serve per testare il riordino. Va documentata la provenienza e la licenza di ogni file.

## 11. Spike da fare per primi (usa-e-getta)

1. **Ricerca**: FTS5 contro Tantivy su 100.000 tracce con errori di battitura, per scegliere il motore.
2. **MeTube**: verificare come si invia un download, come si legge lo stato e se il file finito si recupera in modo affidabile via HTTP.
3. **Scrittura tag**: lofty su tutti i formati elencati, controllando che i file restino leggibili dalle app comuni e che non si perdano dati (copertine incorporate, tag sconosciuti).
4. **Riconoscimento**: per quanti file "stile YouTube" AcoustID + MusicBrainz trovano la registrazione giusta, quanto spesso viene scelta l'edizione giusta, e con quali pesi e soglie il punteggio di confidenza di Armonika (§8.2) separa bene le corrispondenze giuste da quelle sbagliate. Uscita: pesi e soglie scritti nel piano, con i numeri misurati.
5. **Impronta del contenuto**: verificare che l'impronta usata per riconoscere un file spostato da fuori (§8.1) resti uguale quando cambiano solo i tag, su tutti i formati elencati, e quanto costa calcolarla durante la scansione.
6. **Decodifica e impronta audio per AcoustID**: rusty-chromaprint contro la libreria C originale, e per Opus (e HE-AAC) libopus tramite adapter contro un decoder in Rust puro contro ffmpeg. Uscita: la combinazione scelta decodifica tutti i formati accettati e produce impronte che AcoustID riconosce come quelle di `fpcalc`, con tempi di calcolo e impatto sulla compilazione multipiattaforma misurati.

Gli spike non diventano codice di produzione: ciò che si impara entra nel piano.

## 12. Lasciato al builder

- Scelte di libreria specifiche non fissate qui (es. sqlx o rusqlite, osservazione delle cartelle, generazione OpenAPI, tipo di impronta del contenuto).
- Schema esatto del database, purché rispetti il Glossario, §6 e §8 (identificativi stabili delle tracce separati dai file, dati personali legati alle tracce, tracce mancanti ed eliminate, registro annullamenti, librerie e permessi come in §8.5).
- Design visivo dell'interfaccia, nel rispetto del brand kit.
- Dettaglio dell'autenticazione (§7.4), con il vincolo di compatibilità Subsonic.
- Quali endpoint OpenSubsonic implementare per primi, in base a ciò che usano Symfonium e Feishin.
