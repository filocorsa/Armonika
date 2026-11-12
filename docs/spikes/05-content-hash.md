# Spike 5: impronta del contenuto

Data: 2026-11-12. Obiettivo: capire se l'impronta del contenuto (hash dei pacchetti audio compressi, senza tag, senza decodificare; design par. 8.1 e 8.7 punti 1 e 5) è stabile quando cambiano solo i tag, quanto costa e **quando** conviene calcolarla (durante la scansione o in background). Tutti i numeri vengono da esecuzioni reali di `spikes/05-content-hash/`.

## 1. Versioni e dati di prova

- `symphonia` 0.6.1 con `features = ["all"]` (demuxer usati: mp3, flac, ogg, isomp4; nessun decoder viene creato). `blake3` 1.8.7, `lofty` 0.25.5 per le modifiche dei tag, Rust 1.99.1, Windows 11. Linux e macOS non provati.
- Libreria disordinata `bench/data/messy`: 54 file (13 flac, 6 m4a, 15 mp3, 14 ogg Vorbis, 6 opus), 483.746.793 byte, media 8,96 MB per file. Si lavora solo su copie in `bench/data/spike05/` (ignorata da git). I modelli di silenzio non sono stati toccati.
- **Opus**: il demuxer Ogg di Symphonia 0.6.1 legge senza problemi i flussi Opus (anche se Symphonia non ha un decoder Opus): 8.443 pacchetti per `nin-ghosts-01-opus.opus`. Nessun ripiego necessario.
- Disco: SSD (Samsung 990 PRO), **non** un HDD. Cache del sistema operativo: non si può svuotare su Windows senza strumenti esterni, quindi i tempi sono "cache calda o quasi" (vedi par. 4). L'HDD si stima con una formula.

## 2. Come è calcolata l'impronta

Il programma apre il file con `MediaSourceStream` (avvolto in un lettore che conta i byte letti dal disco), lascia che Symphonia faccia il probe del contenitore, prende la prima traccia audio e usa `next_packet()`. Ogni pacchetto entra in BLAKE3 preceduto dalla sua lunghezza (4 byte), così lo spostamento del confine tra due pacchetti cambia l'hash. Tag, copertine, padding, atomi `moov`/`udta`, ID3v1/v2 e pagine di intestazione Ogg non entrano mai nell'hash perché non sono pacchetti audio. L'hash è mostrato troncato a 128 bit (32 caratteri esadecimali) nelle tabelle di lavoro; in produzione si può tenere intero.

- **Variante A (completa)**: tutti i pacchetti.
- **Variante B (campionata)**: 16 pacchetti dall'inizio, 16 dopo un salto (`seek`, modalità `Coarse`) a metà durata, gli ultimi 16 dopo un salto al 98% della durata, più la durata e il numero di frame letti dall'intestazione del contenitore (`track.duration`, `track.num_frames`). Totale 48 pacchetti (45 su flac, dove i frame sono grandi).

Il numero totale di pacchetti e la dimensione totale compressa **non** si possono aggiungere alla variante B a costo zero: per conoscerli bisogna leggere l'intero file (mp3, ogg, flac non hanno un indice dei pacchetti). La durata nell'intestazione è l'unico dato di lunghezza gratuito, ed è quello incluso.

## 3. Stabilità

Per ognuno dei 54 file (copia fresca per ogni caso) si calcolano A e B, si applica la modifica con le API **native** di lofty con salvataggio a livello di file (`split_tag`/`merge_tag`; per flac `FlacFile::save_to_path`, come stabilito dallo spike 3), si ricalcolano A e B. Le copertine sono quelle dello spike 3 (300 KB e 4,7 MiB, generate da me).

Celle: file con impronta invariata su file provati (nessuna riga "diversa" salvo dove indicato).

| Modifica | mp3 (15) | flac (13) | ogg (14) | opus (6) | m4a (6) | A | B |
|---|---|---|---|---|---|---|---|
| Titolo cambiato | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Copertina aggiunta 300 KB | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Copertina aggiunta 4,7 MiB | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Copertina aggiunta e poi rimossa | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Padding 64 KiB + titolo | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Padding 0 + titolo (tag in spazio stretto, i dati si spostano) | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Tutti i tag rimossi | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| Rinomina e spostamento in un'altra cartella | 15/15 | 13/13 | 14/14 | 6/6 | 6/6 | stabile | stabile |
| ID3v1 aggiunto (solo mp3) | 15/15 | n/a | n/a | n/a | n/a | stabile | **instabile: 14/15** |
| ID3v1 aggiunto e poi rimosso (solo mp3) | 15/15 | n/a | n/a | n/a | n/a | stabile | **instabile: 14/15** |

Note:

- **Variante A: nessuna differenza in 462 prove (8 modifiche x 54 file, più 2 modifiche ID3v1 x 15 mp3).** Anche con m4a, dove le modifiche di `ilst` (con 4,7 MiB di copertina) ingrandiscono `moov` e possono spostare `mdat`: i pacchetti restano uguali.
- **Variante B su mp3: instabile quando cambia l'ID3v1** (2 casi su 30 con ID3v1: `nin-ghosts-11-copy.mp3` con ID3v1 aggiunto, `nin-ghosts-02-copy.mp3` con ID3v1 tolto; il secondo file lo aveva già). Causa probabile (dedotta dai risultati, non verificata nel codice di Symphonia): il salto `Coarse` negli mp3 senza indice usa un rapporto sulla lunghezza totale del file in byte, che include i 128 byte finali dell'ID3v1; il punto di salto cambia di qualche pacchetto e il campione di metà e fine cambia. Padding e dimensione dell'ID3v2 invece non disturbano (il rapporto parte dall'inizio dell'audio). Un campionamento fatto "a mano" per offset (relativi all'inizio e alla fine dei soli dati audio) richiederebbe un parser dei frame mp3 nostro.
- Non provato: riordino dei blocchi di metadati FLAC (lofty non lo espone, non lo ho forzato a mano).
- **Audio diverso, impronta diversa.** Su 54 file ci sono 49 impronte A distinte. I 5 gruppi con più file sono tutte coppie `*-compilation` e `*-copy` della stessa sorgente (vedi sotto). Nessun'altra collisione.
- **Originale contro `duplicate_other_format`: 15 coppie, 0 con la stessa impronta A** (come atteso: audio ricodificato, byte diversi).

### Caso interessante: copie in raccolta

`compilation_copy` ha audio identico alla sorgente e tag diversi, quindi **l'impronta è uguale** (corretto: stessi byte audio). Coppie con la stessa impronta, sia in A sia in B:

`nin-slip-03-compilation.flac == nin-slip-03-copy.flac`, `nin-slip-10-compilation.ogg == nin-slip-10-copy.ogg`, `nin-ghosts-05-compilation.flac == nin-ghosts-05-copy.flac`, `bradsucks-mess-05-compilation.mp3 == bradsucks-mess-05-copy.mp3`, `woodward-heretoday-05-compilation.ogg == woodward-heretoday-05-copy.ogg`.

Conseguenza per §8.7 punto 1 (ricollegamento automatico solo se l'impronta del contenuto è uguale a quella di **esattamente una** traccia mancante della stessa libreria): se la libreria ha due file con lo stesso audio (una copia nell'album e una nella raccolta) e uno dei due viene spostato a mano, l'impronta corrisponde a due tracce, quindi la regola non si applica e il ricollegamento automatico non scatta. Il caso diventa un elemento da rivedere, o si risolve con la regola di spareggio per nome del file proposta dal design (stesso nome, come criterio aggiuntivo, non l'impronta da sola). La regola "esattamente una" è quindi giusta e va tenuta: l'impronta del contenuto non può distinguere due copie identiche, e non deve farlo.

## 4. Costi

Misura (`cost`): per ogni file si apre, si fa il probe e si calcola A e B, contando i byte realmente letti dal disco (richieste di lettura di Symphonia, comprese quelle del probe dei tag in testa). Passata 1: ogni file prima con B poi con A (così A non scalda la cache per B); poi 3 passate "calde" intercalate, di cui si riporta la mediana per formato.

**Stato della cache: calda o quasi.** I file erano stati appena copiati con `fs::copy` (nessuna possibilità di svuotare la cache su Windows). La passata 1 non è quindi "a freddo": i valori di B in passata 1 (circa 7 ms per file anche per mp3 che legge 0,19 MB) sono molto più alti della mediana calda (0,26 ms) probabilmente per il costo della prima apertura di ogni file (cache dei metadati, controllo antivirus del sistema), ma è un'ipotesi, non l'ho verificata.

| Formato | n | dimensione media MB | A: ms (passata 1) | B: ms (passata 1) | A: MB letti | B: MB letti | A: ms (caldo, mediana) | B: ms (caldo, mediana) |
|---|---|---|---|---|---|---|---|---|
| flac | 13 | 21,58 | 31,08 | 7,85 | 21,55 | 1,010 | 24,02 | 1,32 |
| m4a | 6 | 3,75 | 8,83 | 6,83 | 3,84 | 0,284 | 6,46 | 0,50 |
| mp3 | 15 | 5,77 | 9,04 | 7,04 | 5,77 | 0,188 | 7,95 | 0,26 |
| ogg | 14 | 5,71 | 14,02 | 7,70 | 5,79 | 0,625 | 14,97 | 0,80 |
| opus | 6 | 2,40 | 6,04 | 7,69 | 2,48 | 0,531 | 5,30 | 0,66 |
| **tutti** | 54 | 8,96 | 15,28 | 7,45 | 8,99 | 0,548 | 13,2 (media pesata delle mediane) | 0,73 (idem) |

A legge l'intero file (MB letti = dimensione). B legge in media 0,55 MB (6% del file) e meno di 1,1 MB nel caso peggiore (flac): sono le letture a blocchi del probe (tag e intestazioni) più un blocco per ogni punto di campionamento, indipendenti dalla lunghezza del brano.

### Estrapolazione a 50.000 file da 8 MB su HDD

Formula: `tempo_per_file = MB_letti / 150 MB/s + N_posizionamenti x 10 ms`, `totale = 50.000 x tempo_per_file`. Il lavoro di CPU si assume sovrapposto alla lettura (A: BLAKE3 e demux costano 13 ms a caldo su questa macchina, meno dei 53 ms di lettura).

| Variante | MB letti per file | Posizionamenti per file (ipotesi) | Tempo per file | 50.000 file | Contro la soglia di 5 minuti |
|---|---|---|---|---|---|
| A (completa) | 8,0 | 1 | 53 + 10 = 63 ms | 3.167 s = **52,8 min** | 10,6 volte la soglia |
| B (campionata) | 0,55 (misurato) | 3 (probe e inizio, metà, fine; ipotesi: 0,19 MB dell'mp3 sono circa 3 blocchi da 64 KiB) | 3,7 + 30 = 33,7 ms | 1.683 s = **28 min** | 5,6 volte la soglia |
| Solo posizionamento di apertura (1 x 10 ms, lettura dei tag che la scansione fa comunque) | circa 0,06 (probe tag) | 1 | 10 ms | 500 s = 8,3 min | già oltre la soglia da sola |

I posizionamenti per file (1 per A, 3 per B) sono un'ipotesi, non una misura: dipendono dalla frammentazione del disco e dal modo in cui l'OS gestisce la lettura anticipata. Il numero di 10 ms e i 150 MB/s vengono dal brief. Su SSD, con cache calda, i totali misurati sarebbero circa 11 minuti (A, 13,2 ms x 50.000) e 37 secondi (B, 0,73 ms x 50.000), prima di qualsiasi parallelismo tra core, che non ho misurato.

Conclusione dei numeri: **nessuna delle due varianti sta nei 5 minuti su HDD durante la prima scansione**, e B costa comunque almeno 3 posizionamenti per file, quasi 3 volte quelli che la scansione fa già per leggere i tag. A è il costo di rileggere l'intera libreria (circa 400 GB).

## 5. Robustezza della variante B

Collisione per campionamento (`collision`): due file da 60 s identici tranne un'onda sinusoidale (440 Hz contro 880 Hz) tra 10 s e 20 s, cioè fuori dai primi 16 pacchetti, dal campione di metà e dagli ultimi 16. Generati da ffmpeg (mp3 CBR 128 kbit/s, flac, ogg Vorbis q4):

| Formato | Dimensioni dei due file (byte) | A uguale? | B uguale? |
|---|---|---|---|
| mp3 CBR | 960.932 e 960.932 | no | **sì** |
| flac | 153.155 e 164.011 | no | **sì** |
| ogg Vorbis | 24.601 e 27.225 | no | **sì** |

B dà quindi un falso "uguale" ogni volta che due file differiscono solo fuori dai punti campionati. Può succedere con silenzio iniziale e finale o con brani che hanno la stessa introduzione e lo stesso finale (non misurato su file reali: nella libreria disordinata non ci sono casi, la prova è sintetica). Includere la durata dall'intestazione (già fatto in B) non basta: nell'esempio la durata è identica. La dimensione totale compressa distinguerebbe flac e ogg (qui 153.155 contro 164.011 byte e 24.601 contro 27.225), ma **non l'mp3 CBR** (stessa dimensione), e comunque non è nota senza leggere o conoscere tutto il file (non è gratuita come "dimensione del file", che include i tag e cambia con essi). Per la regola di §8.7 punto 1 un falso "uguale" è pericoloso: due file diversi con lo stesso campione sembrerebbero la stessa traccia e il ricollegamento unirebbe tracce diverse (con preferiti e playlist). Con B l'impronta può servire al più come **filtro** (scartare candidati con B diverso), mai come prova di uguaglianza.

## 6. Decisione: quando calcolarla

1. **Variante A (completa), BLAKE3 sui pacchetti compressi letti da Symphonia, senza decodifica. In background dopo la scansione**, come l'impronta Chromaprint (§8.7 punto 5), non durante la scansione. Motivo: A è l'unica abbastanza sicura per il ricollegamento automatico (nessun falso "uguale" per campionamento, stabile in 462 prove su tutti i formati (8 modifiche x 54 file, più 2 modifiche ID3v1 x 15 mp3)) ma richiede di leggere tutto il file: 52,8 minuti stimati su HDD per 50.000 file da 8 MB, contro i 5 minuti della prima scansione.
2. **La variante B non va scelta come impronta** (non sta nel budget comunque: 28 minuti stimati su HDD per i soli posizionamenti, è instabile sugli mp3 quando cambia l'ID3v1, ed è soggetta a collisioni da campionamento). Non si usa neppure come impronta rapida in scansione.
3. La scansione registra per ogni file solo ciò che costa già una lettura (percorso, dimensione, data di modifica, tag). L'impronta del contenuto viene calcolata dal processo in background insieme (o poco prima) di Chromaprint; il ricollegamento per spostamento (§8.7 punto 1) usa l'impronta A e funziona per i file la cui impronta è già stata calcolata. Un file spostato prima che la sua impronta sia calcolata si tratta come file nuovo e traccia mancante: è il costo accettato.
4. Il calcolo in background può riusare la lettura che già fa Chromaprint (stesso file letto una volta per le due impronte): da verificare in fase di implementazione, non misurato qui.
5. Nota di design emersa dalla prova: due copie identiche dello stesso audio (copia in raccolta) hanno la stessa impronta, quindi non si ricollegano automaticamente (vedi par. 3). La regola "esattamente una traccia mancante" di §8.7 punto 1 è corretta così com'è.

## 7. Come riprodurre

Da `spikes/05-content-hash/` (servono `bench/data/messy`, le copertine di `spikes/03-tags/work/` e ffmpeg nel PATH):

```
cargo build --release
mkdir -p ../../bench/data/spike05
./target/release/spike-content-hash probe ../../bench/data/messy/nin-ghosts-01-opus.opus
./target/release/spike-content-hash stability ../../bench/data/messy ../../bench/data/spike05/stab
./target/release/spike-content-hash cost ../../bench/data/messy ../../bench/data/spike05/cost
./target/release/spike-content-hash collision ../../bench/data/spike05/coll
```

Su Windows l'eseguibile è `target\release\spike-content-hash.exe`. I tempi dipendono dalla macchina e dallo stato della cache; i totali di stabilità e le collisioni no.
