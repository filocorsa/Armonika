# Fixtures

A 4 second mono tone (a chord of three sines at 261.63, 329.63 and 392 Hz), encoded
once with ffmpeg 6.1.1 so that tests never need ffmpeg. The WAV and AIFF versions
are synthesized by the tests themselves.

```sh
SRC="-f lavfi -i aevalsrc=0.25*sin(2*PI*261.63*t)+0.25*sin(2*PI*329.63*t)+0.25*sin(2*PI*392*t):s=44100:d=4"
C="-ac 1 -map_metadata -1 -fflags +bitexact -flags:a +bitexact"
ffmpeg -y $SRC $C -c:a libmp3lame -b:a 48k tone.mp3
ffmpeg -y $SRC -ar 16000 -sample_fmt s16 $C -c:a flac -compression_level 8 tone.flac
ffmpeg -y $SRC $C -c:a libvorbis -q:a 0 tone.ogg
ffmpeg -y $SRC -af aresample=48000 $C -c:a libopus -b:a 24k tone.opus
ffmpeg -y $SRC $C -c:a aac -b:a 48k tone.m4a
```

The FLAC is at 16 kHz to keep it small; the M4A keeps ffmpeg's normal edit list.
