#!/bin/sh
# Builds a small synthetic audiobook library covering every shape the scanner must handle
# (SPEC §4.3). Invented titles and people only: safe to use in tests, screenshots and docs.
#
# usage: tools/demo/make-library.sh OUT_DIR
set -eu

out=${1:?usage: make-library.sh OUT_DIR}
command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

ff() { ffmpeg -nostdin -hide_banner -loglevel error -y "$@"; }

# A short "speech-like" signal of $1 seconds.
tone() { echo "sine=f=$2:sample_rate=22050,volume='0.4+0.3*sin(2*PI*3*t)':eval=frame"; }

ff -f lavfi -i "color=c=0x3a5a7a:s=600x600:d=1" -frames:v 1 "$work/cover.png"

# m4b with chapters, store-style tags and an embedded cover.
m4b() { # dir name seconds asin title series part extra_metadata...
  dir="$out/$1"; mkdir -p "$dir"
  secs=$3; third=$((secs * 1000 / 3))
  printf ';FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=%d\ntitle=Opening Credits\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=%d\nEND=%d\ntitle=Chapter One\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=%d\nEND=%d\ntitle=Chapter Two\n' \
    "$third" "$third" $((third * 2)) $((third * 2)) $((secs * 1000)) > "$work/ch.txt"
  # ffmpeg can't write custom MP4 tags (series, part, audible_asin) and cover art in one file:
  # with COVER=1 the book gets an embedded cover and standard tags only.
  if [ "${COVER:-0}" = 1 ]; then
    cover="-map 2:v -c:v copy -disposition:v attached_pic"; tagmode=""
  else
    cover=""; tagmode="-movflags use_metadata_tags"
  fi
  # shellcheck disable=SC2086
  ff -f lavfi -i "$(tone "$secs" 220)" -i "$work/ch.txt" -i "$work/cover.png" -t "$secs" \
     -map 0:a $cover -map_metadata 1 -map_chapters 1 -c:a aac -b:a 64k \
     -metadata title="$5" -metadata artist="Ada Example" -metadata album_artist="Ada Example" \
     -metadata composer="Ben Reader, Cass Voice" -metadata album="$5: $6, Book $7 (Unabridged)" \
     -metadata genre="Fantasy, Adventure" -metadata date=2021 -metadata audible_asin="$4" \
     -metadata series="$6" -metadata part="$7" -metadata comment="A storm rises over the harbour." \
     $tagmode -f mp4 "$dir/$2"
}

m4b "The Copper Tide [B0DEMO0001]" "The Copper Tide [B0DEMO0001].m4b" 30 B0DEMO0001 "The Copper Tide" "Harbour Lights" 2
m4b "Harbour Dawn [B0DEMO0002]" "Harbour Dawn [B0DEMO0002].m4b" 24 B0DEMO0002 "Harbour Dawn" "Harbour Lights" 1
m4b "Harbour Prequel [B0DEMO0003]" "Harbour Prequel [B0DEMO0003].m4b" 12 B0DEMO0003 "Harbour Prequel" "Harbour Lights" 0

# Two identical sync-conflict copies and no plain file.
COVER=1 m4b "Twin Tale [B0DEMO0004]" "Twin Tale [B0DEMO0004].m4b-1754152438.952120-AbCdEfGh.m4b" 18 B0DEMO0004 "Twin Tale" "Twins" 1
cp "$out/Twin Tale [B0DEMO0004]/Twin Tale [B0DEMO0004].m4b-1754152438.952120-AbCdEfGh.m4b" \
   "$out/Twin Tale [B0DEMO0004]/Twin Tale [B0DEMO0004].m4b-1754153859.068224-AbCdEfGh.m4b"

# Numbered MP3 parts three levels deep, with an info file naming the reader.
parts="$out/Old Series MP3/Far Station (1999)/Far Station"
mkdir -p "$parts"
for n in 1 2 3 4 5; do
  ff -f lavfi -i "$(tone 6 $((200 + n * 20)))" -t 6 -ac 1 -c:a libmp3lame -b:a 64k \
     -metadata album="Far Station" -metadata artist="Ray Author" -metadata track="$n/5" -metadata genre="Sci-Fi" \
     -metadata title="Far Station 0$n-05" "$parts/Far Station 0$n-05.mp3"
done
printf 'Far Station (1999)\nRead by Ray Example\nUnabridged\n' > "$out/Old Series MP3/Far Station (1999)/info.txt"

# Disc folders under a parent with no audio of its own.
for d in 1 2; do
  mkdir -p "$out/Big Book/CD$d"
  for t in 1 2; do
    ff -f lavfi -i "$(tone 5 300)" -t 5 -ac 1 -c:a libmp3lame -b:a 64k -metadata album="Big Book" \
       -metadata artist="Dee Author" -metadata disc="$d" -metadata track="$t" "$out/Big Book/CD$d/0$t.mp3"
  done
done

# A show: one episode per folder, sentinel part tags, purchase-order part numbers.
for e in "1:3" "2:1" "10:2"; do
  ep=${e%%:*}; part=${e##*:}
  dir="$out/Ep. $ep [B0DEMO01$ep]"; mkdir -p "$dir"
  ff -f lavfi -i "$(tone 8 260)" -t 8 -c:a aac -b:a 64k -metadata title="Ep. $ep: Night $ep" \
     -metadata artist="Eve Writer" -metadata series="Night Shift" -metadata part="$part" \
     -metadata album="Night Shift" -metadata genre="Thriller" -metadata audible_asin="B0DEMO01$ep" \
     -movflags use_metadata_tags -f mp4 "$dir/Ep. $ep [B0DEMO01$ep].m4b"
done

# Podcast episodes (MP3, HTML descriptions) with a trailer.
for item in "Season 1 - SHOW - E1 - Start:B0DEMO0201" "Season 1 - SHOW - E2 - Middle:B0DEMO0202" "SHOW Season 2 Trailer:B0DEMO0203"; do
  title=${item%%:*}; asin=${item##*:}
  dir="$out/$title [$asin]"; mkdir -p "$dir"
  ff -f lavfi -i "$(tone 7 280)" -t 7 -c:a libmp3lame -b:a 128k -metadata title="$title" -metadata album="SHOW" \
     -metadata genre="Podcast" -metadata "lyrics-eng=<p><strong>$title</strong> &amp; more</p>" "$dir/$title [$asin].mp3"
done

# A loose file at the root, a Unicode/colon name, a broken file, a sidecar-only cover.
ff -f lavfi -i "$(tone 6 330)" -t 6 -c:a libmp3lame -b:a 96k -metadata title="Loose Track" "$out/Loose Track.mp3"
uni="$out/Writer’s Tale: Book 1 (Opens 10∕31!) [B0DEMO0301]"; mkdir -p "$uni"
ff -f lavfi -i "$(tone 6 350)" -t 6 -c:a aac -b:a 64k -metadata title="Writer’s Tale" -metadata artist="G. Example" \
   -movflags use_metadata_tags -f mp4 "$uni/Writer’s Tale [B0DEMO0301].m4b"
mkdir -p "$out/Broken [B0DEMO0401]"
head -c 4096 /dev/urandom > "$out/Broken [B0DEMO0401]/Broken [B0DEMO0401].m4b"
mkdir -p "$out/Sidecar Cover [B0DEMO0501]"
ff -f lavfi -i "$(tone 6 370)" -t 6 -c:a libmp3lame -b:a 64k -metadata title="Sidecar Cover" -metadata artist="H. Example" \
   "$out/Sidecar Cover [B0DEMO0501]/Sidecar Cover.mp3"
ff -f lavfi -i "color=c=0x7a3a3a:s=500x500:d=1" -frames:v 1 "$out/Sidecar Cover [B0DEMO0501]/cover.jpg"

# Hostile entries the walk must refuse: a symlink out of the library, a disguised playlist.
ln -sf /etc/passwd "$out/escape.mp3"
mkdir -p "$out/Disguised [B0DEMO0601]"
printf '#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nfile:///etc/passwd\n#EXT-X-ENDLIST\n' \
  > "$out/Disguised [B0DEMO0601]/Disguised.m4b"

echo "demo library written to $out"
