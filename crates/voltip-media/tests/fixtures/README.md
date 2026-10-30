# voltip-media test files

Every file holds the same two seconds: 1.5 s of a 440 Hz sine at half scale, then 0.5 s of
silence (`tests/decode.rs` checks the length, where the silence starts and the frequency). Each is
under 64 KB. They were made with FFmpeg 6.1.1 (`/usr/bin/ffmpeg` on Ubuntu 24.04) by this script,
run in this directory:

```bash
#!/usr/bin/env bash
set -euo pipefail
ff() { /usr/bin/ffmpeg -nostdin -hide_banner -loglevel error -y "$@"; }
# 1.5 s of a 440 Hz sine at half scale, then 0.5 s of silence; a second argument adds a channel.
tone() { echo "aevalsrc=if(lt(t\,1.5)\,0.5*sin(2*PI*440*t)\,0)${2:-}:s=$1:d=2"; }
video="color=c=black:s=16x16:r=5:d=2"
ff -f lavfi -i "$(tone 12000)" -c:a pcm_s16le tone.wav
ff -f lavfi -i "$(tone 12000)" -c:a adpcm_ms adpcm.wav
ff -f lavfi -i "$(tone 44100)" -c:a libmp3lame -b:a 32k tone.mp3
ff -f lavfi -i "$(tone 22050)" -c:a aac -b:a 32k -f adts tone.aac
ff -f lavfi -i "$(tone 48000 '|if(lt(t\,1.5)\,0.5*sin(2*PI*440*t)\,0)')" -c:a aac -b:a 48k tone.m4a
ff -f lavfi -i "$(tone 16000)" -c:a flac -sample_fmt s16 tone.flac
ff -f lavfi -i "$(tone 48000)" -c:a libvorbis -q:a 0 tone.ogg
ff -f lavfi -i "$(tone 48000)" -c:a libopus -b:a 24k tone.opus
ff -f lavfi -i "$video" -f lavfi -i "$(tone 48000)" -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k tone.mp4
ff -f lavfi -i "$video" -f lavfi -i "$(tone 44100)" -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k tone.mov
ff -f lavfi -i "$video" -f lavfi -i "$(tone 16000)" -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a flac -sample_fmt s16 tone.mkv
ff -f lavfi -i "$video" -f lavfi -i "$(tone 48000)" -c:v libvpx -b:v 20k -c:a libopus -b:a 24k tone.webm
ff -f lavfi -i "$video" -f lavfi -i "$(tone 48000)" -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a ac3 -b:a 64k ac3.mkv
ff -f lavfi -i "$video" -c:v libx264 -preset ultrafast -pix_fmt yuv420p video-only.mp4
```

| File | Container | Audio | Also |
|---|---|---|---|
| `tone.wav` | WAV | 16-bit PCM, 12 kHz mono | |
| `adpcm.wav` | WAV | Microsoft ADPCM, 12 kHz mono (not decoded: the error names it) | |
| `tone.mp3` | MP3 | 44.1 kHz mono, 32 kb/s | |
| `tone.aac` | ADTS (raw AAC) | AAC, 22.05 kHz mono | no length in the file; the priming stays in |
| `tone.m4a` | MP4 (M4A) | AAC, 48 kHz stereo (both channels the same) | |
| `tone.flac` | FLAC | 16-bit, 16 kHz mono | |
| `tone.ogg` | Ogg | Vorbis, 48 kHz mono | |
| `tone.opus` | Ogg | Opus, 48 kHz mono (pre-skip 312) | |
| `tone.mp4` | MP4 | AAC, 48 kHz mono | H.264 video |
| `tone.mov` | QuickTime | AAC, 44.1 kHz mono | H.264 video |
| `tone.mkv` | Matroska | FLAC, 16 kHz mono | H.264 video |
| `tone.webm` | WebM | Opus, 48 kHz mono | VP8 video |
| `ac3.mkv` | Matroska | AC-3 (not decoded: the error names it) | H.264 video |
| `video-only.mp4` | MP4 | none (the error says so) | H.264 video |

AAC in Matroska decodes too, but its first frame is the encoder's priming, which Matroska does
not record (an MP4's edit list does, `src/mp4.rs`): it starts about 21 ms late, more than the
one-frame bound these files are held to, so `tone.mkv` carries FLAC. A raw AAC stream (`tone.aac`)
records it nowhere either; its test allows for it.

`-nostdin` matters when the script is piped into `bash`: without it the first `ffmpeg` reads the
rest of the script as its input.
