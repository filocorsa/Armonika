# Fase 2, blocco A: motore di riconoscimento e taratura

Data: 2026-11-17
Stato: bozza, da approvare dall'utente.
Documento principale: [2026-11-12-armonika-design.md](2026-11-12-armonika-design.md). I termini (file, traccia, registrazione, edizione, album, accredito, gruppo) hanno il significato del suo Glossario.

## 1. Perché

Una libreria anche solo di 1000 canzoni ha centinaia di album. Nessuno conferma 400 volte "giusto": se il riordino richiede un click per album, Armonika non convince nessuno a provarla. Il riordino quasi automatico è il punto di forza del progetto. *(Detto dall'utente il 2026-11-17.)*

Ma un errore automatico su **titolo o artisti** è l'errore che l'utente non perdona, in particolare un feat fuso con l'artista principale in un solo artista, che rovina ricerca e navigazione. Un **album sbagliato** è tollerabile, purché Armonika si accorga del dubbio e lo dica. *(Detto dall'utente.)*

La fase 2 è troppo grande per un solo piano, quindi si divide in blocchi, ognuno con il suo design e il suo piano. Questo è il primo, scelto dall'utente: prima di costruire interfaccia e coda si verifica, sui file veri dell'utente, che il riconoscimento regge davvero.

## 2. Cosa costruisce il blocco A

1. **Impronta audio Chromaprint** calcolata in background per le tracce delle librerie, salvata nel database.
2. **Client AcoustID e MusicBrainz** (rete, limiti, cache delle risposte).
3. **Il motore di riconoscimento**: candidati, indizi, i due punteggi (§4), la regola "forma o sostanza" (§5) e la decisione su cosa si applicherebbe (§6).
4. **Un comando di prova a vuoto**, `armonika recognize --dry-run`, che lavora sulle tracce di una libreria e **non scrive niente nei file**. Produce un rapporto leggibile e un pacchetto di dati per la taratura (§7).
5. **Strumenti di taratura** in `bench/`: il programmino di giudizio (§8) e la taratura che calcola pesi e soglie (§9).
6. **Libreria disordinata allargata** con i casi che oggi mancano (§10).

### Obiettivi
- Sapere, con numeri misurati sulla libreria vera dell'utente, quante tracce Armonika sistemerebbe da sola, quante finirebbero in "album da verificare" e quante resterebbero all'utente.
- Nessuna proposta con titolo o artisti sbagliati sopra la soglia di applicazione automatica, sul campione giudicato.
- Pesi e soglie in un file di dati versionato, ricalcolabili con un comando, senza rifare richieste in rete.

### Non-obiettivi (arrivano nei blocchi successivi)
- Scrivere tag, rinominare o spostare file; registro e annullamento.
- Interfaccia web, API nativa, coda, caricamento dal browser.
- Applicare davvero qualcosa in automatico: qui si dice solo cosa si **applicherebbe**.
- Controllo doppioni, copertine dal Cover Art Archive, testi.

### Anti-obiettivi
- Soglie scelte "a occhio" o tarate solo sulla libreria di prova sintetica: per i casi con più artisti valgono i giudizi sui file veri.
- Una logica di punteggio scritta due volte (prodotto e strumento di taratura): il punteggio esiste solo nel motore, lo strumento lo chiama.
- Dati della libreria privata dell'utente nella repository (elenchi di brani, giudizi, risposte in cache): restano in cartelle ignorate da git. Nella documentazione vanno solo numeri aggregati.

## 3. Vincoli

- Tutto il §7.2 del design principale: Symphonia + `opus-pure` + rusty-chromaprint (`preset_test2`, primi 120 s), ffmpeg come ripiego solo se presente (HE-AAC), User-Agent `Armonika/<versione> ( <url del progetto> )`, MusicBrainz al massimo 1 richiesta al secondo, AcoustID al massimo 3, indirizzo di MusicBrainz configurabile.
- **Chiave AcoustID**: il progetto deve registrare la propria applicazione su acoustid.org (lo fa l'utente con il suo account; azione richiesta prima della prova sulla libreria vera). La chiave resta sostituibile in configurazione (`ARMONIKA_ACOUSTID_KEY` o file). Mai scritta nei log, nei rapporti o nella cache.
- Senza internet il comando finisce con un errore chiaro; il server continua a funzionare come in fase 1.
- Regola R35 (registro della fase 1): il codice di questo blocco non va su `main` prima che `docs/phase-1-acceptance.md` sia su `main`; durante la settimana di uso quotidiano (B6, fino a circa il 2026-11-24) il server di casa resta sull'immagine attuale. Il motore, le tabelle nuove e il calcolo in background fanno parte del prodotto (migrazioni sul database vero), non di un eseguibile separato: la prova a vuoto sulla libreria vera si fa **dopo B6**, con la nuova immagine. Fino ad allora si lavora con la libreria disordinata e con risposte di rete registrate nei test. *(Scelta di Claude: un eseguibile separato con un database privato sarebbe lavoro buttato.)*
- Prima della prova sulla libreria vera serve la chiave AcoustID registrata dall'utente (punto sopra): è un passo del piano che aspetta l'utente.
- Il calcolo dell'impronta in background non deve peggiorare le soglie della fase 1 (§3 del design principale): bassa priorità, dopo l'impronta del contenuto, interrompibile.

## 4. I due punteggi

Lo spike 4 usava un punteggio solo, per la coppia registrazione + edizione. Qui se ne usano due, perché i due errori pesano in modo diverso per l'utente.

### 4.1 Fiducia nella registrazione (decide titolo e artisti)
Indizi di partenza (pesi iniziali dallo spike 4, da ritarare, §9):
- **A**: `score` AcoustID del risultato che contiene la registrazione (sotto 0,5 scartato).
- **D**: durata, `max(0, 1 - |durata del file - durata della registrazione| / 10 s)`. Se MusicBrainz non dà la durata, D si **esclude** (valore e peso), non vale 0 (correzione chiesta dallo spike 4).
- **T**: somiglianza tra titolo del file **ripulito** e titolo della registrazione (§5).
- **R**: corrispondenza degli **artisti** (§5.2), non più una somiglianza tra due stringhe intere.
- **I**: nuovo, l'**ISRC**. Se il file ha un ISRC e MusicBrainz lo collega alla registrazione candidata, I = 1; se lo collega a un'altra registrazione, I = 0; senza ISRC, I si esclude. spotDL scrive di solito l'ISRC: per la libreria dell'utente è probabilmente l'indizio più forte. Oggi la scansione non legge l'ISRC: va aggiunto alla lettura dei tag (con l'aumento di `TAG_SCHEMA_VERSION` previsto dalla fase 1). Gli ISRC di una registrazione candidata si chiedono nella stessa richiesta che legge la registrazione (`inc=isrcs` insieme a edizioni e accrediti), quindi senza richieste in più. Se AcoustID non dà risultati e il file ha un ISRC, si fa una richiesta `/isrc/<codice>` e le registrazioni trovate diventano candidate. *(Scelte di Claude.)*
- Regole di sicurezza dello spike che restano: AcoustID sotto 0,9 non supera la soglia di applicazione automatica, salvo ISRC concorde; un file senza titolo **e** senza artista non supera mai la soglia automatica. Questi limiti stanno nel file di dati come parametri, ma la taratura **non li cerca**: si cambiano solo con una decisione esplicita, scritta nel documento della taratura. Pesi e soglie invece sono cercati dalla taratura (§9). *(Scelta di Claude.)*

### 4.2 Fiducia nell'album e nell'edizione (decide album, anno, numero di traccia, copertina)
Due livelli, perché il caso più comune non è "album sbagliato" ma "stesso album, edizione incerta" (normale, deluxe, giapponese):
- **Fiducia nell'album** (release group): la registrazione scelta compare in quali album? Indizi: l'album originale (tipo `Album` senza tipi secondari, il vecchio O), somiglianza del titolo dell'album nel file, gli altri file dello stesso gruppo che puntano allo stesso album, indizi di raccolta (tag `TCMP`/`cpil`, "Various Artists"), regole §11 punti 7 e 11 del design principale.
- **Fiducia nell'edizione** (release) dentro l'album scelto: quanti file del gruppo coprono l'edizione, numero di tracce, edizione già presente nella libreria, data, formato. "Già presente nella libreria" vuol dire: altre tracce della stessa libreria che hanno già nei tag l'identificativo di quell'edizione (letti dal database).

### 4.3 Gruppi
Le tracce di una libreria si raggruppano come gli elementi in coda (§8.7 punto 15 del design principale): stessa cartella **e** stessi tag di titolo dell'album e artista dell'album; una traccia senza album è un gruppo da sola. Il punteggio del gruppo per un album o un'edizione è calcolato come nello spike 4 (media sui file), separatamente per i due livelli. *(Scelta di Claude.)*

## 5. Titolo e artisti: forma o sostanza

Quando la registrazione è sicura, Armonika adotta titolo e artisti di MusicBrainz, perché è l'unico modo di avere i feat come artisti separati, con i loro identificativi. Ma una differenza tra file e proposta deve essere **solo di forma**; una differenza **di sostanza** è un indizio di registrazione sbagliata e manda la traccia all'utente. *(Decisione di Claude, delegata dall'utente il 2026-11-17.)*

### 5.1 Titolo
- Normalizzazione su entrambi i lati come nello spike 4 (NFKD, senza accenti, minuscole, non alfanumerici come spazi).
- Pulizia **solo del titolo del file**: parentesi "di contorno" (official, video, audio, lyric, hd, hq, 4k, visualizer, mv), parti "feat./ft./featuring/con ..." (che diventano artisti candidati, §5.2), suffissi di versione aggiunti dai negozi ("- Remastered 2011", "- 2011 Remaster", "(Remastered)"), prefisso "Artista - ".
- Dopo la pulizia: titoli uguali (o con somiglianza sopra una soglia da tarare) = differenza di forma. Altrimenti = sostanza.
- Attenzione: "(instrumental)", "(live)", "(acoustic)", "(remix)" e simili nel titolo di **MusicBrainz** non si tolgono mai, e se il file non li ha (o viceversa) è sostanza: sono registrazioni diverse.
- L'elenco esatto dei suffissi è lasciato al builder, purché ogni voce sia coperta da un test e l'elenco stia nello stesso file di dati dei pesi (§9).

### 5.2 Artisti
- **Artisti del file**: si ricavano da tutte le fonti sicure: tag multi-valore (`ARTISTS`), identificativi MusicBrainz se presenti, separatori sicuri di §8.1 del design principale (`feat.`, `ft.`, `featuring`, `;`), le parti feat. tolte dal titolo, e il separatore `/` **solo** in ID3v2.3 e solo se ogni parte ha almeno 3 caratteri (regola R32 del registro della fase 1, che lascia a questa fase i file di spotDL). Mai su `&`, `and`, `e`, virgola.
- **Artisti della proposta**: la lista dell'accredito MusicBrainz (ogni artista con il suo identificativo e il testo che lo unisce al successivo).
- Corrispondenza, con nomi normalizzati e alias MusicBrainz dell'artista:
  - ogni artista del file deve comparire nella proposta. Un artista del file assente dalla proposta = **sostanza**;
  - un artista della proposta assente dal file è ammesso solo se il file non ha nessun artista oppure se il suo nome compare altrove nei tag del file (titolo, artista dell'album). Altrimenti = **sostanza**. È la regola prudente: aggiungere un artista che il file non nomina da nessuna parte lo decide l'utente. La taratura dirà se è troppo severa;
  - un accredito che nel file è una stringa unica ("Takagi & Ketra") e in MusicBrainz è un artista unico con lo stesso nome = forma.
- R (§4.1) vale 1 se gli artisti corrispondono senza sostanza, altrimenti un valore più basso (formula al builder, tarata in §9).

## 6. Decisione per traccia

Per ogni traccia il motore produce una di queste decisioni, con i motivi in chiaro:

| Decisione | Quando | Cosa si applicherebbe |
|---|---|---|
| **Applico tutto** | registrazione sopra soglia, nessuna sostanza, edizione sopra soglia | titolo, artisti, album, anno, numero di traccia, identificativi |
| **Applico, edizione da verificare** | registrazione sopra soglia, album sopra soglia, edizione sotto | titolo, artisti, titolo e artista dell'album, anno originale, identificativi di registrazione e album; numero di traccia e identificativo dell'edizione no |
| **Applico, album da verificare** | registrazione sopra soglia, album sotto soglia | titolo, artisti, identificativo della registrazione; i tag dell'album del file restano com'erano; se il file non ha album, la traccia resta senza album (andrebbe tra i "Singoli" del suo artista, §8.7 punto 10 del design principale). Le proposte di album restano pronte da scegliere |
| **A te** | registrazione sotto soglia, oppure sostanza in titolo o artisti | niente |
| **Non riconosciuta** | nessun candidato | niente |

*(Decise con l'utente il 2026-11-17: titolo e artisti contano più dell'album; meglio nessun album che un album sbagliato.)*

Il motivo di ogni decisione è un elenco di frasi brevi e stabili (es. "titolo uguale dopo la pulizia", "artista X del file non nell'accredito", "ISRC concorde", "possibile raccolta"), così il rapporto, il programmino di giudizio e in futuro l'interfaccia mostrano la stessa spiegazione.

## 7. Il comando di prova a vuoto

`armonika recognize --dry-run --library <nome> [--out <cartella>]`:
- usa le impronte già calcolate dal lavoro in background (che fa parte di questo blocco, §2 punto 1); per le tracce che non le hanno ancora le calcola al momento e le salva;
- interroga AcoustID e MusicBrainz rispettando i limiti, con avanzamento visibile; le risposte vanno in una cache nel database o nella cartella `data`, così un secondo giro non rifà le richieste;
- non scrive nei file della libreria e non cambia i dati musicali né l'identità delle tracce nel database; scrive solo impronte audio e cache;
- scrive nella cartella di uscita:
  - un **rapporto** leggibile (riepilogo per decisione, poi una riga per traccia con decisione e motivi);
  - un **pacchetto di taratura**: per ogni traccia i tag letti, tutti i candidati con **tutti gli indizi grezzi** (non solo i punteggi) e la decisione. Con questo la taratura ricalcola i punteggi senza rete e senza i file musicali.
- Durata attesa: per i 172 file della libreria dell'utente, pochi minuti; per 50.000 file molte ore (1 richiesta al secondo a MusicBrainz). Il comando lo dice all'inizio con una stima.
- Ricerca testuale su MusicBrainz quando AcoustID non dà risultati e il file ha titolo e artista: necessaria, altrimenti molti file finirebbero "non riconosciuti". I suoi `score` non entrano come confidenza (§8.2 del design principale).

Formato esatto di rapporto e pacchetto, e dove sta la cache (tabella del database o file), sono lasciati al builder.

## 8. Programmino di giudizio

Strumento da terminale in `bench/` (come `armonika-bench`), che gira anche su Windows e legge solo il pacchetto di taratura.

- Mostra una proposta alla volta: a sinistra i tag attuali, a destra la proposta, differenze evidenziate; sotto punteggi, decisione e motivi.
- Tasti *(chiesti dall'utente; `n` e `m` sono scelte di Claude)*:
  - `g` giusta;
  - `s` sbagliata, poi un secondo tasto: `1` titolo o artisti, `2` album o edizione, `3` entrambi (*scelta di Claude*: serve a tarare separatamente i due punteggi);
  - `n` non so: la proposta non entra nella taratura;
  - freccia sinistra oppure `i`: torna indietro e cambia il giudizio;
  - `m`: apre la pagina MusicBrainz della proposta nel browser;
  - `q`: esce.
- Salva dopo ogni tasto in un file di giudizi accanto al pacchetto; riaprendo riprende dal primo non giudicato.
- **Campione**: l'utente giudica 100-150 proposte (accordato il 2026-11-17). Il programmino le sceglie stratificate: tutte le decisioni rappresentate, più peso alle proposte vicine alle soglie e a quelle con più artisti, perché lì si decide la taratura. Se la libreria ha meno proposte del campione, le mostra tutte.
- Per ascoltare un brano dubbio l'utente usa la sua app (Feishin); il programmino non riproduce audio.

## 9. Taratura

Strumento in `bench/` che legge uno o più pacchetti con i rispettivi giudizi (libreria vera dell'utente, libreria disordinata con la sua verità nota) e:
- ricalcola i punteggi **chiamando il motore** (stessa libreria di codice, nessuna copia della formula);
- cerca pesi e soglie come nello spike 4 (griglia, ordine di scelta esplicito), ma separatamente per registrazione e per album/edizione;
- vincolo principale: **zero proposte con titolo o artisti sbagliati sopra la soglia della registrazione** sul campione giudicato; inoltre la soglia deve stare almeno **3 punti** sopra il punteggio più alto di una registrazione sbagliata, e la controprova (pesi scelti senza una parte del campione, provati su quella parte) non deve mettere registrazioni sbagliate sopra soglia. A parità, la soglia che lascia meno tracce all'utente. Lo stesso, separatamente, per album ed edizione. *(Scelta di Claude.)*
- fonti dei giudizi: la verità nota della libreria disordinata conta come giudizi già dati; i giudizi dell'utente sono un'altra fonte. Il vincolo principale deve valere **su ciascuna fonte da sola**, e il rapporto dà i numeri per fonte e per tracce con più artisti, perché per quei casi decide la libreria vera (§2);
- onestà sui numeri: zero errori su circa 130 giudizi non dimostra zero errori (con quel campione un tasso d'errore vero fino a circa il 2-3% passerebbe inosservato). Il documento della taratura lo scrive; il registro e l'annullamento del blocco successivo sono la rete di sicurezza;
- scrive i pesi e le soglie scelti in un **file di dati versionato** nella repository (letto dal motore all'avvio, non costanti nel codice), più un rapporto con i numeri: errori sopra soglia, quanti "applico tutto", "da verificare", "a te", controprova lasciando fuori una parte del campione.

Esito scritto in `docs/recognition-calibration.md` (solo numeri aggregati, nessun nome di brano della libreria privata).

## 10. Libreria disordinata allargata

La libreria di prova dello spike 4 non ha feat, accrediti con più artisti né raccolte vere: i casi che contano di più per l'utente. Si allarga con:
- brani Creative Commons con **più artisti nell'accredito** (feat. e collaborazioni), da trovare su MusicBrainz con licenza verificata come oggi;
- almeno una **raccolta vera**;
- danni nuovi: feat. nel titolo invece che negli artisti, `/` in ID3v2.3 come spotDL, suffissi "Remastered", ISRC presente e assente;
- le correzioni già note di `gen-messy` (lettura e scrittura con le API native di lofty, che non perdono `DISCC`/`MCN`/`TOTALTRACKS`; nomi di file neutri in cartelle per edizione, vedi `bench/messy/README.md`).

Se per qualche caso non si trovano brani CC adatti, il caso si copre solo con la libreria vera e lo si scrive nel documento della taratura.

## 11. Uscita del blocco A

Il blocco è finito quando:
1. il motore, la prova a vuoto e gli strumenti sono su un ramo con test verdi (e su `main` quando R35 lo permette);
2. la prova a vuoto è stata fatta sulla libreria vera dell'utente e sulla libreria disordinata allargata;
3. l'utente ha giudicato almeno 100 proposte;
4. la taratura ha prodotto pesi e soglie che rispettano i vincoli di §9 (criterio a numeri, non a giudizio), e `docs/recognition-calibration.md` dice quante tracce Armonika sistemerebbe da sola;
5. il design principale è aggiornato: §8.2 e §8.6 punto 4 ("applicazione automatica non nella prima versione") sostituiti dalla regola dei due punteggi di questo documento, con i numeri misurati.

Se il punto 4 non si raggiunge (troppi errori sopra qualsiasi soglia utile), si torna dall'utente prima di costruire il blocco successivo.

## 12. Approcci considerati

1. **Tutto nel prodotto, tarato a mano**: pesi come costanti, ogni giro di taratura è una modifica al codice. Scartato: la taratura non è riproducibile e ogni giro costa una PR.
2. **Nuovo spike in Python, poi porting in Rust**: scartato perché la stessa logica scritta due volte diverge proprio nei dettagli (normalizzazione, artisti) che decidono se un feat diventa artista separato.
3. **Motore nel prodotto più strumenti di taratura** (scelto dall'utente il 2026-11-17): una sola implementazione del punteggio, pesi in un file di dati, taratura rifacibile senza rete.

## 13. Lasciato al builder

- In quale crate vive il motore (il design principale prevede `armonika-organizer`; si può iniziare con una crate dedicata al riconoscimento se l'organizer non esiste ancora), purché il motore non dipenda dal server e sia richiamabile dagli strumenti in `bench/`.
- Schema delle tabelle per impronte audio e cache delle risposte.
- Formato di rapporto, pacchetto, giudizi e file dei pesi.
- Libreria per l'interfaccia da terminale del programmino.
- Formula esatta di R e del punteggio dell'album, purché tarata in §9.
