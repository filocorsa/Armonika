# Spike 7: prova di un file HE-AAC

Data: 2026-11-16. Obiettivo (design par. 7.2 e 11 punto 1): provare un M4A HE-AAC di musica con licenza libera. Tre domande: `probe::read_file` legge tag e durata? `content_hash` funziona? Symphonia decodifica il livello AAC-LC di base, e a quale frequenza e durata? **Esito: non è stato possibile procurarsi un file HE-AAC verificato, quindi le tre misure sono rinviate alla fase 2.** Il programma in `spikes/07-he-aac/` non è stato scritto, perché non c'è un file su cui eseguirlo.

## 1. Cosa è stato provato per procurarsi il file

- **Codificatori locali** (`ffmpeg`, Windows 11): solo `aac` (nativo, solo LC) e `aac_mf` (MediaFoundation). `libfdk_aac` assente. `ffmpeg -h encoder=aac_mf` non mostra opzioni di profilo. Prova: `ffmpeg -t 20 -i nin-ghosts-02-m4a.m4a -c:a aac_mf -b:a 24k -ac 1 out.m4a`, poi `ffprobe`: profilo **LC**, 44100 Hz. Il codificatore Windows produce solo LC anche a bitrate bassissimo.
- **File già presenti** in `bench/data` (6 m4a in `messy`, più un m4a ALAC in `decode-extra`): tutti LC (o ALAC), nessuno HE-AAC.
- **File pubblicati online**: i file di prova HE-AAC (AOT 5 e 29) della pagina di Fraunhofer (`www2.iis.fraunhofer.de/AAC/stereo.html`) sono toni di prova, non musica, e la pagina non dichiara alcuna licenza: non usati. Il campione HE-AAC di Wikimedia Commons (CC0, voce, non musica) risulta non più raggiungibile (404). Commons non accetta m4a. Nessun m4a HE-AAC di musica con licenza Creative Commons trovato.
- **ffmpeg con `libfdk_aac`**: non scaricato. Quel codificatore ha una licenza non ridistribuibile (già escluso dal design per l'adapter) e un build da fonti non verificate non vale il rischio per una prova.

## 2. Limiti

Nessuna delle tre misure è stata eseguita. Ipotesi non verificata, da non considerare risultato: Symphonia dovrebbe leggere un HE-AAC come AAC-LC a metà frequenza di campionamento (ad esempio 22050 Hz per un file a 44100 Hz) e con la durata dichiarata nel contenitore, ma non è misurato.

## 3. Esito e cosa resta

HE-AAC resta **non garantito**, come deciso nel design par. 11 punto 1 (ripiego su ffmpeg se presente, altrimenti file senza impronta audio e riconosciuto dai tag). La decisione sulla scelta di decodifica non cambia. Rinvio alla fase 2: ripetere la prova se arriva un file HE-AAC reale (per esempio dalla libreria di un utente), con `probe::read_file`, `content_hash` e una decodifica Symphonia.
