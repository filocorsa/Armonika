# Spike della fase 0

Gli spike sono prove usa-e-getta (design §11): il codice sta in `spikes/` ed è escluso dal workspace, il prodotto è il documento qui sotto. Gli esiti sono riportati nel design (`docs/superpowers/specs/2026-11-12-armonika-design.md`) con la data 2026-11-12; le questioni lasciate all'utente sono elencate in §11 del design.

| Spike | Documento | Esito |
|---|---|---|
| 1. Motore di ricerca | [01-search.md](01-search.md) | Tantivy: p95 massimo circa 4 ms e recall@10 0,97 con un errore di battitura su 100.000 tracce; FTS5 supera i 50 ms sugli errori di battitura. Dati sintetici, da rimisurare su una libreria vera in fase 1. |
| 2. MeTube | [02-metube.md](02-metube.md) | Recupero via HTTP affidabile senza cartella condivisa: 20 download su 20, 12 nomi difficili su 12, ripresa dopo l'arresto. MeTube non ha autenticazione propria. |
| 3. Scrittura dei tag | [03-tags.md](03-tags.md) | lofty con API nativa split/merge: mp3, flac, ogg, opus scrivibili in sicurezza; m4a solo senza metadati `keys`/`mdta`; wav e aiff proposti in sola lettura. Mai l'API generica. |
| 4. Riconoscimento | [04-recognition.md](04-recognition.md) | Punteggio `100 * (A + 2D + T + R + O) / pesi presenti`, "incerta" sotto 85, conferma in blocco da 90; 54/54 registrazioni giuste sul campione, ma soglie non validate: da rimisurare in fase 2. |
| 5. Impronta del contenuto | [05-content-hash.md](05-content-hash.md) | BLAKE3 su tutti i pacchetti compressi, stabile in 462 prove; calcolata in background dopo la scansione (circa 53 minuti stimati per 50.000 file su HDD). |
| 6. Decodifica e impronta audio | [06-decode-fingerprint.md](06-decode-fingerprint.md) | Symphonia per tutto tranne Opus, `opus-pure` per Opus, rusty-chromaprint: 57/57 file decodificati, 20/20 stesso risultato AcoustID di `fpcalc`, nessuna dipendenza C. Scelta condizionata alla domanda su HE-AAC (non provato). |
