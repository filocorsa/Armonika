# Spike 3: scrittura dei tag con lofty su tutti i formati

Data: 2026-11-12. Obiettivo: capire se la libreria `lofty` scrive i tag dentro i file senza rovinarli (design: i tag del file sono l'unica fonte dei dati musicali; la copertina incorporata e i tag sconosciuti non devono andare persi). Tutto ciò che segue viene da esecuzioni reali del programma in `spikes/03-tags/`, non dalla documentazione.

## 1. Versioni e dati di prova

- `lofty` 0.25.5 (ultima versione pubblicata il 2026-11-12, `cargo search lofty`), Rust 1.99.1, `edition = "2024"`.
- ffmpeg e ffprobe 6.1.1 (build gyan.dev), Windows 11. Linux e macOS non provati: `lofty` è Rust puro, ma i risultati sotto sono misurati solo su Windows.
- I file di partenza sono copie dei modelli di silenzio in `bench/templates/` (0,2 s, senza tag; i modelli non vengono mai toccati, le copie stanno in `spikes/03-tags/work/out/`, ignorata da git). Formati: mp3, flac, ogg (Vorbis), opus, m4a (AAC), wav, aiff.
- Copertine, generate da me con ffmpeg da un'immagine di prova più rumore (nessuna immagine di terzi), comandi in `spikes/03-tags/make_covers.sh`:
  - `cover_300k.jpg`: 800x800, `-q:v 3`, 304.043 byte (sha256 che inizia con `abb995169981cde0`).
  - `cover_5m.jpg`: 2600x2600, `-q:v 1`, 4.943.241 byte (circa 4,7 MiB, sha256 che inizia con `80b33283da312d1d`).

## 2. Come è fatta la prova

Per ogni coppia formato x caso (`spikes/03-tags/src/main.rs`):

1. Si parte dal modello. Per `unknown_tag` si scrive prima con ffmpeg `-metadata ARMONIKA_TEST=keepme` (verificato con ffprobe che ci sia davvero).
2. Si scrive il caso con lofty. Il caso `basic` scrive titolo, artista, album, artista dell'album, traccia 3 di 12, disco 1 di 2, data 2021-05-14. `mbids` scrive gli identificativi MusicBrainz di registrazione, traccia dell'edizione, edizione, album, due artisti, artista dell'album, canzone. `multi_artist` scrive `ARTIST` più `ARTISTS` multi-valore. `cover` e `large_cover` scrivono la copertina JPEG da 300 KB e da 5 MB. `unicode` scrive accenti, cirillico, greco, giapponese, coreano ed emoji.
3. Si rilegge con lofty e si confronta ogni valore atteso (anche l'ordine dei valori multipli e i byte della copertina).
4. Si legge con `ffprobe -show_format -show_streams`, si decodifica con `ffmpeg -v error -i <file> -f null -` (deve uscire con codice 0 e senza output), si confronta l'MD5 dell'audio decodificato (`ffmpeg -map 0:a:0 -f md5 -`) con quello del file di partenza.
5. Si modifica solo il titolo e si ripete tutto: ogni tag nativo diverso dal titolo (letto con le API native di lofty, un elemento per riga), il tag sconosciuto, l'hash della copertina, la decodifica e l'MD5 dell'audio devono risultare invariati.

Stati: **ok** (tutto invariato e riletto), **perso** (almeno un valore, il tag sconosciuto o la copertina non c'è più o non si può scrivere), **corrotto** (lofty dà errore, ffprobe o la decodifica falliscono, o l'audio decodificato cambia). Un `perso` può essere un limite del contenitore (il formato non ha il campo) oppure una vera perdita di dati: le note dicono quale.

Ho provato anche due varianti che non stavano nel piano: `mp3-v23` (ID3v2.3 invece di 2.4, con `WriteOptions::use_id3v23`) e, per wav e aiff, **entrambi** i contenitori di tag che lofty sa scrivere. Ho provato **due modi di usare lofty**:

- **API generica** (`lofty::read_from_path`, modifica del `Tag` generico, `save_to_path`): è quella mostrata negli esempi.
- **API nativa con split/merge** (si apre il file col suo tipo, si prende il tag nativo, `split_tag()`, si modifica il `Tag` generico, `merge_tag()`, si scrive): è quella da usare in Armonika (vedi §5).

## 3. Risultati

Tabella con l'API nativa split/merge, su tutti i casi del piano. Nessuna cella è **corrotto**: in nessuna esecuzione lofty ha dato errore, ffprobe e ffmpeg hanno sempre letto e decodificato i file, e l'MD5 dell'audio decodificato è sempre identico a quello di partenza, dopo la scrittura e dopo la modifica del titolo.

| Formato (contenitore di tag) | basic | mbids | multi_artist | unknown_tag | cover | large_cover | unicode |
|---|---|---|---|---|---|---|---|
| mp3 (ID3v2.4) | ok | ok | ok | ok | ok | ok | ok |
| mp3 (ID3v2.3) | ok | ok | ok | ok | ok | ok | ok |
| flac (Vorbis comment) | ok | ok | ok | ok (1) | ok | ok | ok |
| ogg Vorbis (Vorbis comment) | ok (1) | ok (1) | ok (1) | ok (1) | ok (1) | ok (1) | ok (1) |
| opus (Vorbis comment) | ok (1) | ok (1) | ok (1) | ok (1) | ok (1) | ok (1) | ok (1) |
| m4a (ilst) | ok | ok | ok | perso (2) | ok | ok | ok |
| wav, tag ID3v2 (scelta predefinita di lofty) | ok | ok | ok | ok (3) | ok | ok | ok |
| wav, RIFF INFO | perso (4) | perso (4) | perso (4) | perso (4) | perso (4) | perso (4) | ok |
| aiff, tag ID3v2 (scelta predefinita di lofty) | ok | ok | ok | ok | ok | ok | ok |
| aiff, chunk di testo AIFF | perso (4) | perso (4) | perso (4) | perso (4) | perso (4) | perso (4) | perso (4) |

Note:

1. In ogg, opus e flac il commento `ENCODER` scritto da ffmpeg (`Lavc libvorbis`, `Lavc libopus`, `Lavf...`) sparisce alla prima scrittura (la stringa del produttore, `VENDOR`, resta invariata; dal codice di lofty il commento `ENCODER` è mappato su `ItemKey::EncoderSoftware`, ma non ho approfondito perché non venga riscritto). È solo informativo, ma è un tag che prima c'era: l'ho segnato come nota e non come `perso`.
2. Il tag sconosciuto di m4a l'ho potuto creare con ffmpeg **solo** con `-movflags use_metadata_tags`, che scrive i metadati nel formato `keys`/`mdta` (di default ffmpeg scarta le chiavi sconosciute in m4a). lofty non legge quel formato (`native_before` vuoto) e alla scrittura l'etichetta `ARMONIKA_TEST` sparisce da ffprobe, mentre nel file resta l'atomo `keys` vecchio accanto al nuovo `ilst`: una struttura ibrida. Con il formato normale (atomo libero `----:com.apple.iTunes:ARMONIKA_TEST`, quello che usano iTunes e Picard) il tag sconosciuto sopravvive: l'ho verificato come caso aggiuntivo `unknown_freeform`, creato con l'API nativa di lofty per imitare un altro programma (`ok`, anche dopo la modifica del titolo). Vedi §6.
3. ffmpeg non sa scrivere un tag personalizzato in un wav (`-write_id3v2 1` viene ignorato e il tag viene scartato): per questo caso il `TXXX:ARMONIKA_TEST` è stato scritto con l'API nativa di lofty prima della prova. Per il wav con RIFF INFO ho scritto allo stesso modo un chunk `IZZZ`. In aiff ffmpeg con `-write_id3v2 1` ci riesce, quindi lì il tag viene da ffmpeg.
4. Limite del contenitore, non bug: `lofty` rifiuta (`Tag::push` restituisce falso) i campi che il contenitore non prevede. RIFF INFO non ha artista dell'album, numero disco, identificativi MusicBrainz, `ARTISTS`, né copertine; i chunk di testo AIFF hanno solo nome, autore, copyright e commenti (niente album, numero traccia, anno). Quei valori semplicemente non vengono scritti. Il titolo e gli altri campi che il contenitore ha vengono scritti e riletti correttamente.

Confronto con l'API generica sul tag sconosciuto (stessi file, stesso `ARMONIKA_TEST=keepme`, stessa modifica del solo titolo):

| Formato | API nativa split/merge | API generica (`read_from_path` + `Tag`) |
|---|---|---|
| mp3 (2.4 e 2.3), wav ID3, aiff ID3 | ok | ok (il `TXXX` diventa una voce `ItemKey::Unknown`) |
| flac, ogg, opus | ok | **perso**: `ARMONIKA_TEST` sparisce alla prima scrittura |
| m4a, atomo libero | ok | ok |
| wav RIFF INFO (chunk `IZZZ`) | ok | **perso** |

Su un file con tag sconosciuti nei commenti Vorbis (flac, ogg, opus) l'API generica **cancella** quei tag: lofty converte in `Tag` solo ciò che ha una `ItemKey` e riscrive il commento Vorbis da zero. È una perdita silenziosa di dati dell'utente, e il codice di esempio più comune cade proprio lì.

### Un'insidia nella copertina FLAC

In una versione intermedia della prova ho usato lo split/merge ma scrivendo con `VorbisComments::save_to_path`. In FLAC le copertine sono blocchi `PICTURE` del file, non parte del commento Vorbis: dopo la modifica del solo titolo il blocco copertina **era sparito** (`perso`, per `cover` e `large_cover`). Scrivendo invece con `FlacFile::set_vorbis_comments(...)` + `FlacFile::save_to_path(...)` la copertina resta, con lo stesso hash. Nella tabella sopra c'è la versione corretta. In ogg e opus lo split mette le copertine nel `Tag` e il merge le rimette nel commento, quindi non c'è il problema.

## 4. Nomi esatti dei tag per formato

Lettura diretta dei tag nativi nei file scritti (`work/results.json`, campi `native_after_write`). Il nome nativo è quello che lofty scrive per la `ItemKey` generica indicata: per tutti i campi del design **la `ItemKey` generica basta, non servono le API native per scrivere**. I nomi coincidono con la mappatura documentata di MusicBrainz Picard (Picard non è installato qui, quindi non ho confrontato i byte di un file scritto da Picard: è un confronto con la documentazione, non una prova).

| Campo del design | `ItemKey` di lofty | ID3v2 (mp3, wav, aiff) | Vorbis comment (flac, ogg, opus) | MP4 (m4a) |
|---|---|---|---|---|
| titolo | `TrackTitle` | `TIT2` | `TITLE` | `©nam` |
| artista | `TrackArtist` | `TPE1` | `ARTIST` | `©ART` |
| album | `AlbumTitle` | `TALB` | `ALBUM` | `©alb` |
| artista dell'album | `AlbumArtist` | `TPE2` | `ALBUMARTIST` | `aART` |
| numero traccia / totale | `TrackNumber`, `TrackTotal` | `TRCK` = `3/12` | `TRACKNUMBER`, `TRACKTOTAL` | `trkn` (coppia binaria 3, 12) |
| numero disco / totale | `DiscNumber`, `DiscTotal` | `TPOS` = `1/2` | `DISCNUMBER`, `DISCTOTAL` | `disk` (coppia binaria 1, 2) |
| data o anno | `RecordingDate` | `TDRC` (v2.4); `TYER` + `TDAT` (v2.3) | `DATE` | `©day` |
| MusicBrainz registrazione | `MusicBrainzRecordingId` | `UFID` con proprietario `http://musicbrainz.org` | `MUSICBRAINZ_TRACKID` | `----:com.apple.iTunes:MusicBrainz Track Id` |
| MusicBrainz traccia dell'edizione | `MusicBrainzTrackId` | `TXXX:MusicBrainz Release Track Id` | `MUSICBRAINZ_RELEASETRACKID` | `----:com.apple.iTunes:MusicBrainz Release Track Id` |
| MusicBrainz edizione | `MusicBrainzReleaseId` | `TXXX:MusicBrainz Album Id` | `MUSICBRAINZ_ALBUMID` | `----:com.apple.iTunes:MusicBrainz Album Id` |
| MusicBrainz album (release group) | `MusicBrainzReleaseGroupId` | `TXXX:MusicBrainz Release Group Id` | `MUSICBRAINZ_RELEASEGROUPID` | `----:com.apple.iTunes:MusicBrainz Release Group Id` |
| MusicBrainz artista (più valori) | `MusicBrainzArtistId` | `TXXX:MusicBrainz Artist Id` (valori separati da NUL) | `MUSICBRAINZ_ARTISTID` ripetuto | `----:com.apple.iTunes:MusicBrainz Artist Id` (un atomo, due dati) |
| MusicBrainz artista dell'album | `MusicBrainzReleaseArtistId` | `TXXX:MusicBrainz Album Artist Id` | `MUSICBRAINZ_ALBUMARTISTID` | `----:com.apple.iTunes:MusicBrainz Album Artist Id` |
| MusicBrainz canzone (work) | `MusicBrainzWorkId` | `TXXX:MusicBrainz Work Id` | `MUSICBRAINZ_WORKID` | `----:com.apple.iTunes:MusicBrainz Work Id` |
| artisti multipli | `TrackArtists` | `TXXX:ARTISTS` (valori separati da NUL) | `ARTISTS` ripetuto | `----:com.apple.iTunes:ARTISTS` (un atomo, più dati) |
| copertina | `Picture` (`CoverFront`) | `APIC` | flac: blocco `PICTURE`; ogg e opus: `METADATA_BLOCK_PICTURE` | `covr` (nessun tipo di copertina, lofty la rilegge come `Other`) |

Altre osservazioni sulle API:

- ID3v2.3 non ha `TDRC`: con `use_id3v23(true)` lofty scrive `TYER` e `TDAT`. L'intestazione dei file è `ID3\x04` (v2.4, predefinito) o `ID3\x03`.
- ID3v2 non ha un vero multi-valore in v2.3; lofty scrive e rilegge correttamente gli `ARTISTS` anche lì (test `ok`), ma altri programmi potrebbero leggerli diversamente. Non l'ho verificato.
- `wav` e `aiff`: il contenitore predefinito di lofty (`primary_tag_type()` sui modelli) è **ID3v2**, scritto come chunk `ID3 ` dentro il file; ffprobe lo legge e mostra titolo, artista, album, artista dell'album, `track`, `disc`, `date` e anche gli identificativi MusicBrainz e le copertine come flusso video allegato. RIFF INFO (wav) e i chunk di testo (aiff) sono scelte esplicite (`TagType::RiffInfo`, `TagType::AiffText`). ffprobe vede da RIFF INFO titolo, artista, album, traccia, data e (chiave grezza) `IFRM` per il totale delle tracce; dai chunk AIFF solo `title` e `author`.
- I nomi degli atomi MP4 sono quelli letti dal file; la coppia `trkn`/`disk` risulta "Unknown" nell'enumerazione di lofty perché è un dato binario, ma `TrackNumber`/`DiscNumber` si rileggono correttamente.
- Dimensioni: una copertina da 304.043 byte porta il file opus da 192 a 411.096 byte (base64 dentro il commento) e il file ogg da 3.961 a 414.890 byte; in mp3, flac e m4a l'aumento è di circa la dimensione della copertina più qualche decina di byte. La copertina da 4,7 MiB non ha dato problemi in nessun formato.

## 5. Come si deve usare lofty in Armonika

Conclusioni di sicurezza, tutte verificate dai risultati sopra:

1. **Mai l'API generica per scrivere.** Per scrivere si apre il file con il tipo specifico (`MpegFile`, `FlacFile`, `VorbisFile`, `OpusFile`, `Mp4File`, `WavFile`, `AiffFile`), si prende il tag nativo, si fa `split_tag()`, si modifica il `Tag`, si fa `merge_tag()` e si salva. Così i tag senza `ItemKey` restano nel `remainder`.
2. **FLAC: salvare il file intero** (`FlacFile::save_to_path`), non solo il commento Vorbis, altrimenti si perde la copertina.
3. Per tutti i campi del design basta la `ItemKey` generica dentro lo split/merge: nessun caso richiede di scrivere a mano i nomi nativi.
4. Dopo ogni scrittura, nel flusso reale di Armonika, rileggere e confrontare (è quello che fa questo spike) prima di rendere definitivo il cambio; è già nel design come "possibilità di annullare".

## 6. Elenco di uscita: formati scrivibili in sicurezza

**Si possono scrivere in sicurezza** (con l'API nativa split/merge, tutti i campi del design, copertina fino a 5 MB, tag sconosciuti e audio intatti):

- mp3 (ID3v2.4 predefinito; ID3v2.3 disponibile con `use_id3v23`),
- flac,
- ogg Vorbis,
- opus,
- m4a, **con una condizione**: i file con metadati nel formato `keys`/`mdta` (per esempio prodotti da ffmpeg con `use_metadata_tags`, o alcune registrazioni di telefoni) perdono i loro tag sconosciuti e restano con una struttura ibrida. Prima di scrivere bisogna riconoscere un atomo `keys` dentro `meta` e, in quel caso, trattare il file come sola lettura. Non ho verificato quanto siano diffusi questi file nelle librerie reali.

**Da proporre all'utente come "sola lettura" per quel formato** (decisione sua, il design prevede già la sola lettura per formato):

- **wav e aiff.** Tecnicamente lofty li scrive senza danni **solo** nel chunk ID3v2 (tutti i casi `ok`, audio identico). Gli altri contenitori (RIFF INFO, chunk di testo AIFF) non hanno artista dell'album, numero disco, identificativi MusicBrainz, copertina e così via: scriverli perderebbe campi. Resta da decidere se un chunk ID3 dentro wav/aiff è accettabile per l'utente: ffprobe lo legge, ma non ho provato altri lettori (Windows Media, iTunes, Kodi, app Subsonic). La proposta prudente è sola lettura per wav e aiff nella prima versione di Armonika e riesame dopo la prova con i lettori veri.
- Nessun altro formato risulta non sicuro.

Altri limiti noti: il commento `ENCODER` di ogg, opus e flac scritto da ffmpeg viene eliminato alla prima scrittura (informativo); i chunk di testo AIFF contengono byte UTF-8 anche con caratteri non ASCII, mentre la specifica AIFF parla di ASCII (non verificato come li mostrano altri programmi); tutti i risultati vengono da file di 0,2 s di silenzio, non da file reali con padding, ID3v1, APE o tag annidati.

## 7. Riprodurre

Dalla radice del repository, con ffmpeg e ffprobe nel `PATH`:

```
cd spikes/03-tags
sh make_covers.sh
cargo run --release
```

Il programma stampa una riga per coppia formato x caso (`ok`, `perso`, `n/a`, `corrotto`, più le note) e scrive `work/results.json` con, per ogni esecuzione, i tag letti da ffprobe e i tag nativi prima, dopo la scrittura e dopo la modifica del titolo. Le righe `/generic` sono le stesse prove con l'API generica. `n/a` significa che il tag sconosciuto non si è potuto creare con alcun mezzo in quel contenitore (chunk di testo AIFF).
