# Spike 1: motore di ricerca (SQLite FTS5 contro Tantivy)

Decisione: **Tantivy**. FTS5 non rispetta la soglia di latenza sulle query con errore di battitura (p95 78,6 ms contro il limite di 50 ms); Tantivy rispetta tutte le soglie con ampio margine (p95 massimo circa 4 ms). La ricerca resta dietro l'interfaccia `SearchIndex`, quindi il motore e' sostituibile.

## Cosa e' stato misurato

- Dati: `bench/data/big-100k/manifest.json`, 100.000 tracce, 10.677 album, 4.476 artisti (circa il 10% dei nomi con accenti o CJK). Dati sintetici: il vocabolario dei titoli e' piccolo, quindi molte parole compaiono in migliaia di tracce (caso sfavorevole per le liste di occorrenze).
- Cosa si cerca: un'unica ricerca restituisce tre gruppi (tracce, album della libreria, artisti), ognuno con i primi 10 risultati. La latenza misurata e' quella dell'intera ricerca (tre gruppi, recupero dei testi dei risultati e riordino incluso).
- Bersagli: 200, con seed fisso `20261112` (generatore splitmix64, in `src/queries.rs`): 100 tracce, 50 album, 50 artisti. Il 30% di ogni gruppo e' scelto tra i nomi con accenti che si possono togliere (per avere abbastanza query `diacritics`), il resto a caso tra gli altri.
- Query per ogni bersaglio, uguali per entrambi i motori (salvate in `bench/data/search-queries.json`):
  - `prefix`: ogni prefisso del nome digitato, da 3 caratteri al nome intero (3.303 query; `prefix:full` e' solo il nome intero).
  - `typo`: nome completo con un errore (sostituzione, inserzione o cancellazione) nella parola piu' lunga (200).
  - `diacritics`: nome con gli accenti tolti e minuscolo, es. "beyonce" (60, solo i bersagli con accenti).
  - `multiword`: "artista titolo" per le tracce, "artista album" per gli album, nome per gli artisti (200).
- Successo (recall@10): nel gruppo del bersaglio compare, tra i primi 10, un risultato con lo stesso nome (e, per `multiword`, lo stesso artista).
- Riordino comune: i motori producono solo 50 candidati nell'ordine nativo; un passo uguale per tutti (`common.rs`, distanza di edit per parola, prefisso sull'ultima parola, peso maggiore al titolo) sceglie i 10 finali. Le colonne `native@10` (bersaglio nei primi 10 dell'ordine nativo) e `cand@50` (bersaglio tra i 50 candidati) mostrano quanto conta il motore da solo.
- Misure: build `--release`, indice gia' aperto, riscaldamento (una query ogni 15) escluso, ogni query eseguita una volta, p50 e p95 con il metodo del rango piu' vicino. Macchina: AMD Ryzen 5 5600H, 27,9 GB RAM, SSD, Windows 11, rustc 1.99.1. RAM = working set del processo letto con il crate `sysinfo` (su Windows e' il working set), a riposo = subito dopo l'apertura dell'indice, "dopo" = dopo tutte le query. Un processo per motore e variante.
- Versioni: SQLite 3.54.1 (incluso da `rusqlite` 0.42.0, feature `bundled`, `libsqlite3-sys` 0.40.0), Tantivy 0.27.0.
- Rumore: le misure girano su un PC non dedicato. Due esecuzioni complete hanno dato valori entro circa il 15% nelle varianti veloci, ma le varianti lente (trigram su tutti i trigrammi) hanno oscillato fino a 2x. La conclusione non cambia.

## Come sono implementate le varianti

FTS5 (`src/fts5.rs`), tre tabelle per ogni indice (tracce, album, artisti), due file:
- `unicode61 remove_diacritics 2` con `prefix='2 3 4'`: parole intere AND, ultima parola con prefisso (`"tok"*`), ordinamento bm25 con titolo x3.
- `trigram remove_diacritics 1` (disponibile nel SQLite incluso): per i refusi la query e' l'OR dei trigrammi delle parole, ordinati per bm25. Limiti: un errore rovina fino a 3 trigrammi, quindi serve l'OR e non l'AND; i trigrammi comuni hanno liste enormi e il costo e' proporzionale a esse; le parole sotto i 3 caratteri non producono trigrammi. Variante "potata" (`fts5-*`): solo gli 8 trigrammi piu' rari che esistono nell'indice (frequenza letta da una tabella `fts5vocab`); variante `fts5f-*`: tutti i trigrammi.
- `cascade`: prima unicode61; se un gruppo ha meno di 10 risultati si aggiungono quelli del trigram. Il server non sa di che tipo e' la query, quindi la cascata e' la combinazione utilizzabile davvero.
- Accenti: variante `prefold` (default, `fts5-*`/`fts5f-*`) = testo ripulito in Rust (`fold`: accenti, ø, ł, ß, ligature) prima di indicizzare e interrogare, con testo originale in colonne `UNINDEXED`; variante `native` (`fts5n-*`) = solo il tokenizer.

Tantivy (`src/tant.rs`): un indice per gruppo, campi titolo/artista/album con tokenizer `SimpleTokenizer` + `LowerCaser` + `AsciiFoldingFilter`; per ogni parola una `FuzzyTermQuery` (distanza 1 se la parola ha almeno 4 caratteri, altrimenti 0; per l'ultima parola la variante con prefisso) su tutti i campi, parole in AND; in piu' clausole `Should` con termini esatti (con titolo x3), perche' il punteggio dei fuzzy e' costante. Un solo segmento dopo il merge.

## Risultati

Latenze in ms; recall@10 dopo il riordino comune. Valori della sola esecuzione `all` finale.

### Per tipo di query

| motore | tipo | n | p50 | p95 | recall@10 | native@10 | cand@50 |
|---|---|---|---|---|---|---|---|
| FTS5 unicode61 (prefold) | prefix | 3303 | 4,26 | 14,56 | 0,609 | 0,603 | 0,778 |
| FTS5 unicode61 (prefold) | typo | 200 | 0,34 | 0,58 | 0,025 | 0,025 | 0,025 |
| FTS5 unicode61 (prefold) | diacritics | 60 | 0,90 | 4,81 | 1,000 | 1,000 | 1,000 |
| FTS5 unicode61 (prefold) | multiword | 200 | 2,07 | 5,32 | 1,000 | 1,000 | 1,000 |
| FTS5 trigram potata | prefix | 3303 | 22,40 | 77,00 | 0,575 | 0,497 | 0,703 |
| FTS5 trigram potata | typo | 200 | 25,53 | 79,30 | 0,920 | 0,880 | 0,920 |
| FTS5 trigram potata | diacritics | 60 | 14,99 | 26,05 | 0,800 | 0,700 | 0,800 |
| FTS5 trigram potata | multiword | 200 | 24,12 | 39,68 | 0,945 | 0,870 | 0,945 |
| FTS5 cascade potata | prefix | 3303 | 7,43 | 30,81 | 0,609 | 0,603 | 0,778 |
| FTS5 cascade potata | typo | 200 | 23,92 | 78,55 | 0,925 | 0,885 | 0,925 |
| FTS5 cascade potata | diacritics | 60 | 6,30 | 25,64 | 1,000 | 1,000 | 1,000 |
| FTS5 cascade potata | multiword | 200 | 22,47 | 37,36 | 1,000 | 1,000 | 1,000 |
| FTS5 cascade, tutti i trigrammi | prefix | 3303 | 7,58 | 75,07 | 0,609 | 0,603 | 0,778 |
| FTS5 cascade, tutti i trigrammi | typo | 200 | 34,20 | 93,94 | 0,925 | 0,885 | 0,925 |
| FTS5 cascade, tutti i trigrammi | diacritics | 60 | 5,55 | 82,17 | 1,000 | 1,000 | 1,000 |
| FTS5 cascade, tutti i trigrammi | multiword | 200 | 52,45 | 117,43 | 1,000 | 1,000 | 1,000 |
| FTS5 unicode61 (solo tokenizer, native) | diacritics | 60 | 0,89 | 3,43 | 0,900 | 0,900 | 0,900 |
| **Tantivy** | prefix | 3303 | 1,41 | 4,05 | 0,613 | 0,581 | 0,745 |
| **Tantivy** | typo | 200 | 1,30 | 4,05 | 0,970 | 0,945 | 0,970 |
| **Tantivy** | diacritics | 60 | 0,83 | 2,00 | 1,000 | 1,000 | 1,000 |
| **Tantivy** | multiword | 200 | 1,41 | 3,03 | 1,000 | 1,000 | 1,000 |

("potata" = solo gli 8 trigrammi piu' rari.)

### Dettaglio di `prefix` per lunghezza digitata

| motore | 3-4 car. (p95 / recall) | 5-8 car. | 9+ car. | nome intero |
|---|---|---|---|---|
| FTS5 unicode61 | 43,6 ms / 0,231 | 14,6 ms / 0,302 | 12,7 ms / 0,769 | 4,8 ms / 0,995 |
| FTS5 cascade potata | 42,5 ms / 0,231 | 14,6 ms / 0,302 | 30,7 ms / 0,769 | 27,9 ms / 0,995 |
| Tantivy | 3,5 ms / 0,228 | 4,1 ms / 0,305 | 4,1 ms / 0,775 | 3,9 ms / 0,995 |

Il recall basso dei prefissi corti vale per tutti i motori: con 100.000 tracce un prefisso di 3 o 4 caratteri corrisponde a migliaia di tracce e non c'e' alcun segnale di popolarita' per ordinarle. Non e' un criterio della regola di decisione; il prodotto dovra' aggiungere un ordinamento (ascolti, preferiti) per i prefissi corti.

### Costruzione, dimensione, RAM

| motore | build (s) | su disco (MiB) | RAM a riposo (MiB) | RAM dopo le query (MiB) |
|---|---|---|---|---|
| FTS5 unicode61 (prefold) | 1,2 | 26,0 | 7,9 | 13,6 |
| FTS5 cascade (unicode61 + trigram, prefold) | 2,6 | 57,4 | 8,0 | 15,8 |
| FTS5 unicode61 (native) | 0,9 | 20,2 | 7,9 | 13,4 |
| Tantivy | 1,6 | 4,4 | 8,2 | 19,2 |

Nota sulla RAM: il working set include le pagine mappate in memoria e la cache di SQLite (predefinita); le differenze sono piccole rispetto al caso d'uso. Il processo di misura da solo occupa circa 8 MiB. La dimensione su disco di Tantivy e' molto minore perche' comprime i testi memorizzati. Il tempo di build include il merge in un solo segmento (Tantivy) e `optimize` piu' `VACUUM` (FTS5).

## Regola di decisione (dal brief, applicata alla lettera)

Si sceglie FTS5 se p95 < 50 ms per tutti i tipi e recall@10 >= 0,9 per `typo` e `diacritics`; altrimenti Tantivy, se le rispetta; se nessuna le rispetta si scrive cosa manca e si porta la questione all'utente.

- FTS5, scegliendo la combinazione migliore per tipo (`prefix`, `diacritics`, `multiword` con unicode61; `typo` con trigram): `diacritics` recall 1,000 (con testo ripulito in Rust; 0,900 con il solo tokenizer, che non riduce ø e ł), ma `typo` p95 = 79,3 ms (potata) o 94 ms (tutti i trigrammi), oltre 50 ms. Il recall `typo` (0,92) e' sopra 0,9. Con unicode61 da solo `typo` ha recall 0,025. La cascata utilizzabile in produzione ha lo stesso limite (p95 `typo` 78,6 ms). **FTS5 non rispetta la regola.**
- Tantivy: p95 massimo 4,05 ms (< 50), recall `typo` 0,970 e `diacritics` 1,000 (>= 0,9). **Tantivy rispetta la regola, quindi si sceglie Tantivy.**

Perche' FTS5 e' lento sul trigram: l'OR dei trigrammi deve scorrere le liste di occorrenze, e con questi dati anche i trigrammi "rari" compaiono in migliaia di tracce; la potatura ha ridotto la latenza solo in parte (`multiword` da 117 a 37 ms p95, `typo` da 94 a 79 ms). Non e' stato provato un indice di vocabolario separato con distanza di edit (stile spellfix): e' l'unica strada ancora aperta per FTS5 e comporterebbe codice nostro da mantenere; con Tantivy la tolleranza ai refusi e' gia' nel motore.

## Limiti e note oneste

- Dati sintetici (vocabolario piccolo): su una libreria reale i trigrammi sarebbero piu' selettivi e FTS5 potrebbe essere piu' veloce; non e' stato misurato.
- Il tokenizer di Tantivy usato (`SimpleTokenizer`) tratta una sequenza di caratteri CJK come un'unica parola: la ricerca per sottostringa CJK non e' coperta (FTS5 trigram la copre, se la sottostringa ha almeno 3 caratteri). Il testo CJK e' ~un decimo dei nomi nei dati e non e' stato testato come tipo di query a parte.
- Il recall dei prefissi (prefix) non ha soglia; vedi la nota sopra.
- Il riordino comune migliora entrambi i motori; i dati grezzi del motore da solo sono nelle colonne `native@10` e `cand@50`.
- Le latenze sono lato motore (nessuna rete, nessun HTTP); la soglia del design e' lato server.

## Come riprodurre

Dalla radice del repository, con `bench/data/big-100k/manifest.json` presente (vedi Task 4):

```
cd spikes/01-search
cargo run --release -- all
```

Genera le query (seed 20261112), costruisce gli indici sotto `bench/data/` (git-ignorato), esegue le misure (un processo per motore, circa 15 minuti, la maggior parte per le varianti trigram) e stampa le tabelle. Singoli passi: `queries`, `build fts5`, `build tantivy`, `bench <motore>`, `report` (vedi l'intestazione di `src/main.rs`). I risultati grezzi sono in `bench/data/search-results/*.json`.

## Conseguenze per il design

- Implementare `SearchIndex` con Tantivy (un indice per gruppo: tracce, album, artisti), con il passo di riordino comune; indice ricostruibile dal database, quindi niente migrazioni di schema dell'indice.
- Normalizzare accenti con `AsciiFoldingFilter`; valutare un tokenizer per CJK se la libreria lo richiede.
- Per i prefissi corti aggiungere un segnale di popolarita' all'ordinamento.
