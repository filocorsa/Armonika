# Messy library

A small library of real Creative Commons recordings whose metadata and layout are damaged
on purpose, the way real collections are: video style titles, missing tags, missing
covers, editions with slightly different album titles, the same recording in two formats,
compilations, wrong case. It is used to measure identification and duplicate detection
against a known ground truth.

Build it with:

```
cargo run -p armonika-testdata --bin gen-messy -- --manifest bench/messy/manifest.toml --sources bench/data/cc-sources --out bench/data/messy
```

Missing sources are downloaded from `source_url` and checked against `sha256`. The output
folder gets one file per entry of `ground_truth.json`. The manifest format is described in
`manifest.toml`. To regenerate, delete `bench/data/messy` first: `gen-messy` refuses an
output folder that is not empty. Downloads stay in `bench/data/cc-sources` (git-ignored)
and are reused after their hash is checked again.

The current manifest has 34 sources from 4 editions (Nine Inch Nails "Ghosts I-IV" and
"The Slip", Josh Woodward "Here Today", Brad Sucks "Guess Who's a Mess") in FLAC, MP3 and
Ogg Vorbis. With the damages applied the library has 54 files: 34 primary copies, 15 format
duplicates (6 Opus, 6 M4A, 1 MP3, 1 Ogg, 1 FLAC) and 5 compilation copies. Every damage
kind is used at least 5 times; `split_album` is applied to 6 of the 12 "Ghosts I-IV"
tracks. Some sources are left undamaged on purpose as clean controls.

## Editions

| Edition | MusicBrainz release | Download item | License stated by |
| ------- | ------------------- | ------------- | ----------------- |
| Nine Inch Nails, Ghosts I-IV (2008-03-02, digital, 36 tracks) | [3af7ec8c-3bf4-4e6d-9bb3-1885d22b2b6a](https://musicbrainz.org/release/3af7ec8c-3bf4-4e6d-9bb3-1885d22b2b6a) | [nineinchnails_ghosts_I_IV](https://archive.org/details/nineinchnails_ghosts_I_IV) | Nine Inch Nails, official free release under CC BY-NC-SA 3.0 US; the MusicBrainz release links this archive.org item and the license |
| Nine Inch Nails, The Slip (2008-05-05, digital, 10 tracks) | [12b57d46-a192-499e-a91f-7da66790a1c1](https://musicbrainz.org/release/12b57d46-a192-499e-a91f-7da66790a1c1) | [nine_inch_nails_the_slip](https://archive.org/details/nine_inch_nails_the_slip) | Nine Inch Nails, official free release under CC BY-NC-SA 3.0 US |
| Josh Woodward, Here Today (2004, 13 tracks) | [fed79b91-cbd4-4d9c-8261-e7e9b48f3b61](https://musicbrainz.org/release/fed79b91-cbd4-4d9c-8261-e7e9b48f3b61) | [JoshWoodward-HereToday](https://archive.org/details/JoshWoodward-HereToday) | Josh Woodward on Jamendo (album 32625), CC BY 3.0; the MusicBrainz release links that Jamendo album |
| Brad Sucks, Guess Who's a Mess (2012-11-02, digital, 10 tracks) | [e6577867-6876-4f54-98b1-da0db635bd51](https://musicbrainz.org/release/e6577867-6876-4f54-98b1-da0db635bd51) | [jamendo-117853](https://archive.org/details/jamendo-117853) | Brad Sucks on Jamendo (album 117853), CC BY-NC-SA 3.0 per the item's License.txt; the MusicBrainz release links that Jamendo album |

## Provenance and license

The "Recording" column is the MusicBrainz recording id written to the manifest. "AcoustID
confirmed" means that `fpcalc -json` on the downloaded file, looked up on
`https://api.acoustid.org/v2/lookup` with `meta=recordingids`, returned a top result that
contains that recording id (top result score in brackets). Each recording was also looked
up on `https://musicbrainz.org/ws/2/recording/<id>?inc=releases` to check that it appears on
the edition's release. All checks were done on 2026-11-12.

| Source id | Artist | Title | Edition | License | Attribution | Download | Recording | AcoustID confirmed |
| --------- | ------ | ----- | ------- | ------- | ----------- | -------- | --------- | ------------------ |
| nin-ghosts-01 | Nine Inch Nails | 1 Ghosts I | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [01_Ghosts_I.flac](https://archive.org/download/nineinchnails_ghosts_I_IV/01_Ghosts_I.flac) | `b2fdf3ce-78a6-4e01-9a7b-441700e8ea66` | yes (1.00) |
| nin-ghosts-02 | Nine Inch Nails | 2 Ghosts I | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [02_Ghosts_I_320kb.mp3](https://archive.org/download/nineinchnails_ghosts_I_IV/02_Ghosts_I_320kb.mp3) | `1f3f0aed-47ed-42f4-b1b6-f02f2a13437c` | yes (0.99) |
| nin-ghosts-05 | Nine Inch Nails | 5 Ghosts I | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [05_Ghosts_I.flac](https://archive.org/download/nineinchnails_ghosts_I_IV/05_Ghosts_I.flac) | `132a7820-4a29-4745-bf20-222386434f47` | yes (0.97) |
| nin-ghosts-07 | Nine Inch Nails | 7 Ghosts I | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [07_Ghosts_I_320kb.mp3](https://archive.org/download/nineinchnails_ghosts_I_IV/07_Ghosts_I_320kb.mp3) | `1d1bb32a-5bc6-4b6f-88cc-c043f6c52509` | yes (0.99) |
| nin-ghosts-10 | Nine Inch Nails | 10 Ghosts II | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [10_Ghosts_II.flac](https://archive.org/download/nineinchnails_ghosts_I_IV/10_Ghosts_II.flac) | `226f53e6-ec5f-4783-8684-65bc3c85665b` | yes (1.00) |
| nin-ghosts-11 | Nine Inch Nails | 11 Ghosts II | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [11_Ghosts_II_320kb.mp3](https://archive.org/download/nineinchnails_ghosts_I_IV/11_Ghosts_II_320kb.mp3) | `a95eeadf-9c4a-486d-9610-58a41b6f43ed` | yes (0.99) |
| nin-ghosts-15 | Nine Inch Nails | 15 Ghosts II | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [15_Ghosts_II.flac](https://archive.org/download/nineinchnails_ghosts_I_IV/15_Ghosts_II.flac) | `2d7f08e1-be1c-4b86-b725-6e675b7b6de0` | yes (1.00) |
| nin-ghosts-19 | Nine Inch Nails | 19 Ghosts III | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [19_Ghosts_III_320kb.mp3](https://archive.org/download/nineinchnails_ghosts_I_IV/19_Ghosts_III_320kb.mp3) | `1668f841-f56b-4dc6-a0b2-482bd30f98d6` | yes (0.99) |
| nin-ghosts-22 | Nine Inch Nails | 22 Ghosts III | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [22_Ghosts_III.flac](https://archive.org/download/nineinchnails_ghosts_I_IV/22_Ghosts_III.flac) | `c8795f14-8162-4caa-98c2-0d80b1338034` | yes (1.00) |
| nin-ghosts-25 | Nine Inch Nails | 25 Ghosts III | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [25_Ghosts_III_320kb.mp3](https://archive.org/download/nineinchnails_ghosts_I_IV/25_Ghosts_III_320kb.mp3) | `68d1bdc5-4af4-4f16-9277-96cb4c909635` | yes (0.99) |
| nin-ghosts-28 | Nine Inch Nails | 28 Ghosts IV | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [28_Ghosts_IV.ogg](https://archive.org/download/nineinchnails_ghosts_I_IV/28_Ghosts_IV.ogg) | `e491d339-8f23-4941-af6d-d151bd6a3bfd` | yes (0.99) |
| nin-ghosts-36 | Nine Inch Nails | 36 Ghosts IV | Ghosts I-IV | CC BY-NC-SA 3.0 US | Nine Inch Nails, Ghosts I-IV | [36_Ghosts_IV.ogg](https://archive.org/download/nineinchnails_ghosts_I_IV/36_Ghosts_IV.ogg) | `e2c0fc50-94fa-4919-8c64-2dbfc9ec1c07` | yes (0.99) |
| nin-slip-01 | Nine Inch Nails | 999,999 | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [01_999999.flac](https://archive.org/download/nine_inch_nails_the_slip/01_999999.flac) | `30f3f33e-8d0c-4e69-8539-cbd701d18f28` | yes (1.00) |
| nin-slip-02 | Nine Inch Nails | 1,000,000 | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [02_1000000.flac](https://archive.org/download/nine_inch_nails_the_slip/02_1000000.flac) | `99244237-850b-4a93-904d-57305bcadb4e` | yes (1.00) |
| nin-slip-03 | Nine Inch Nails | Letting You | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [03_letting_you.flac](https://archive.org/download/nine_inch_nails_the_slip/03_letting_you.flac) | `b4a88e68-f66e-4155-8ef4-9f9f46944767` | yes (1.00) |
| nin-slip-04 | Nine Inch Nails | Discipline | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [04_discipline.flac](https://archive.org/download/nine_inch_nails_the_slip/04_discipline.flac) | `a58a2b13-70b5-44c2-9fd5-67f93db17de7` | yes (1.00) |
| nin-slip-05 | Nine Inch Nails | Echoplex | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [05_echoplex.flac](https://archive.org/download/nine_inch_nails_the_slip/05_echoplex.flac) | `52646e3f-1fc0-4a1d-a089-08ac33a30973` | yes (1.00) |
| nin-slip-06 | Nine Inch Nails | Head Down | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [06_head_down.mp3](https://archive.org/download/nine_inch_nails_the_slip/06_head_down.mp3) | `b4f9917f-4514-4e3e-aea3-3f901a9c69a7` | yes (1.00) |
| nin-slip-07 | Nine Inch Nails | Lights in the Sky | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [07_lights_in_the_sky.mp3](https://archive.org/download/nine_inch_nails_the_slip/07_lights_in_the_sky.mp3) | `d7d5eb38-bb10-4cd8-9542-f489e9f16e73` | yes (1.00) |
| nin-slip-08 | Nine Inch Nails | Corona Radiata | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [08_corona_radiata.mp3](https://archive.org/download/nine_inch_nails_the_slip/08_corona_radiata.mp3) | `3c55ef28-626b-4cb2-81a6-acc37b189a3f` | yes (1.00) |
| nin-slip-09 | Nine Inch Nails | The Four of Us Are Dying | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [09_the_four_of_us_are_dying.ogg](https://archive.org/download/nine_inch_nails_the_slip/09_the_four_of_us_are_dying.ogg) | `d7cbbdd6-96da-4da5-b01f-53eb72e73d16` | yes (0.98) |
| nin-slip-10 | Nine Inch Nails | Demon Seed | The Slip | CC BY-NC-SA 3.0 US | Nine Inch Nails, The Slip | [10_demon_seed.ogg](https://archive.org/download/nine_inch_nails_the_slip/10_demon_seed.ogg) | `9f06df64-a373-492e-9a23-6a14a3fde97e` | yes (0.99) |
| woodward-heretoday-01 | Josh Woodward | Incoherent | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [01-Incoherent.ogg](https://archive.org/download/JoshWoodward-HereToday/01-Incoherent.ogg) | `e584534f-2090-408c-8518-83d4ea5cbf8a` | yes (0.97) |
| woodward-heretoday-02 | Josh Woodward | Mona Lisa | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [02-MonaLisa.ogg](https://archive.org/download/JoshWoodward-HereToday/02-MonaLisa.ogg) | `095de530-63bc-4680-bc36-c5a2d24f8e3c` | yes (0.97) |
| woodward-heretoday-03 | Josh Woodward | Dust | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [03-Dust.ogg](https://archive.org/download/JoshWoodward-HereToday/03-Dust.ogg) | `e7d29e25-2dcb-45fe-8cc5-c240f1d7d500` | yes (0.97) |
| woodward-heretoday-04 | Josh Woodward | Troublemaker | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [04-Troublemaker.ogg](https://archive.org/download/JoshWoodward-HereToday/04-Troublemaker.ogg) | `36e8c8f8-0a3f-4c71-a307-f4c596d3a248` | yes (0.97) |
| woodward-heretoday-05 | Josh Woodward | House in My Head | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [05-HouseInMyHead.ogg](https://archive.org/download/JoshWoodward-HereToday/05-HouseInMyHead.ogg) | `bfda21cc-5dab-44ab-9754-2b527a666f2b` | yes (0.97) |
| woodward-heretoday-06 | Josh Woodward | Learn to Fly | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [06-LearnToFly.ogg](https://archive.org/download/JoshWoodward-HereToday/06-LearnToFly.ogg) | `df33b640-a729-472e-8737-1aec83eb82a2` | yes (0.97) |
| woodward-heretoday-07 | Josh Woodward | Nothing in the Dark | Here Today | CC BY 3.0 | Josh Woodward, Here Today | [07-NothingInTheDark.ogg](https://archive.org/download/JoshWoodward-HereToday/07-NothingInTheDark.ogg) | `8b897dfd-6925-4461-a969-006bd22c4326` | yes (0.97) |
| bradsucks-mess-01 | Brad Sucks | In Your Face | Guess Who's a Mess | CC BY-NC-SA 3.0 | Brad Sucks, Guess Who's a Mess | [01-1001519-Brad Sucks-In Your Face.mp3](https://archive.org/download/jamendo-117853/01-1001519-Brad%20Sucks-In%20Your%20Face.mp3) | `a2923af5-1571-40b6-b707-04f842ebf503` | yes (1.00) |
| bradsucks-mess-02 | Brad Sucks | Come Back | Guess Who's a Mess | CC BY-NC-SA 3.0 | Brad Sucks, Guess Who's a Mess | [02-1001520-Brad Sucks-Come Back.mp3](https://archive.org/download/jamendo-117853/02-1001520-Brad%20Sucks-Come%20Back.mp3) | `1694727d-30ac-46b5-9f42-6f746b4e5c1b` | yes (1.00) |
| bradsucks-mess-03 | Brad Sucks | Feel Free! Plastic Surgery! | Guess Who's a Mess | CC BY-NC-SA 3.0 | Brad Sucks, Guess Who's a Mess | [03-1001521-Brad Sucks-Feel Free_ Plastic Surgery_.mp3](https://archive.org/download/jamendo-117853/03-1001521-Brad%20Sucks-Feel%20Free_%20Plastic%20Surgery_.mp3) | `2c0ab49c-0a46-4f3a-a39d-130d5fb75afb` | yes (1.00) |
| bradsucks-mess-04 | Brad Sucks | Guess Who's a Mess | Guess Who's a Mess | CC BY-NC-SA 3.0 | Brad Sucks, Guess Who's a Mess | [04-1001522-Brad Sucks-Guess Who_s a Mess.mp3](https://archive.org/download/jamendo-117853/04-1001522-Brad%20Sucks-Guess%20Who_s%20a%20Mess.mp3) | `09fe51b4-0c9c-4aa4-a916-dec24eeba5ed` | yes (1.00) |
| bradsucks-mess-05 | Brad Sucks | Waste of TV | Guess Who's a Mess | CC BY-NC-SA 3.0 | Brad Sucks, Guess Who's a Mess | [05-1001523-Brad Sucks-Waste of TV.mp3](https://archive.org/download/jamendo-117853/05-1001523-Brad%20Sucks-Waste%20of%20TV.mp3) | `536d06f4-45f3-4578-993a-7306f02ec6dc` | yes (1.00) |

## Notes for the spikes that use this library

- Tags of the same edition differ between formats, as they do in real collections: the
  Ghosts FLAC and MP3 files say album "Ghosts I-IV", the Ghosts Ogg files (archive.org
  derivatives) say "Ghosts I - IV", and their titles look like "1 Ghosts I". After
  `split_album` they become "Ghosts I-IV (Deluxe)" and "Ghosts I - IV (Deluxe)".
- The Ogg files of "The Slip" and "Ghosts I-IV" and the MP3 files of "The Slip" are
  archive.org transcodes of the official FLAC files; the MP3s carry a malformed ID3 comment
  frame (ffprobe warns "Incorrect BOM value"). That is also left as found.
- The Josh Woodward files match two MusicBrainz recordings each in AcoustID: the 2004
  "Here Today" recording and the 2008 "Here Today (Remixed)" recording, with the same
  fingerprint. The manifest uses the 2004 edition because that release links the Jamendo
  album the files come from. Recognition should treat either answer as a near miss, not
  as a wrong recording; the confidence score calibration (spike 4) should expect this.
- Some AcoustID top results list more than one recording (for example `nin-ghosts-22` also
  lists the recording of track 26, and `nin-slip-08`, `nin-slip-09`, `nin-slip-10` list
  other recordings of the same songs). The manifest recording is always among them.
- Compilation copies keep the source's recording and release ids in `ground_truth.json`:
  the recording is the same, the album on the file is the fake "Greatest Hits".


## Known limitations

- Mutating damages are written with lofty's generic tag API, which drops some fields on
  mutated copies. Verified: on `nin-ghosts-10.flac` the damaged copy loses DISCC, MCN and
  TOTALTRACKS and writes TRACKTOTAL as "36;36". This undeclared extra damage is not in the
  ground truth. To fix before phase 2 acceptance by switching to the native split/merge API.
- The output is flat and file names reveal the source id and the role (`-copy`, `-opus`,
  `-compilation`), so grouping by source folder (design 8.7 point 15) is not exercised and
  name-based heuristics see the answer. To change before phase 2 acceptance: neutral file
  names, per-edition folders, and the mapping kept in `ground_truth.json`.
- Several sources are archive.org derivatives (`_320kb.mp3`, `.ogg`) that archive.org may
  regenerate, which would fail the sha256 check. Keep a private backup of
  `bench/data/cc-sources`.
