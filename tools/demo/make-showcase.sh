#!/bin/sh
# Builds a small, good-looking synthetic library for screenshots and demos: books across
# several genre shelves, two series, a podcast-style show, chapters and titled covers.
# Invented titles, people and text only. The audio is a quiet tone, not speech.
#
# usage: tools/demo/make-showcase.sh OUT_DIR
set -eu

out=${1:?usage: make-showcase.sh OUT_DIR}
command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

ff() { ffmpeg -nostdin -hide_banner -loglevel error -y "$@"; }
font=$(fc-match -f '%{file}' sans 2>/dev/null || true)

# cover DIR "TITLE" "AUTHOR" COLOUR1 COLOUR2: a gradient with the title and author, if a
# font is available (plain gradient otherwise).
cover() {
  printf '%s' "$2" | fold -s -w 16 > "$work/title.txt"
  printf '%s' "$3" > "$work/author.txt"
  text=""
  if [ -n "$font" ]; then
    text=",drawtext=fontfile=$font:textfile=$work/title.txt:fontsize=64:fontcolor=white:line_spacing=12:x=56:y=120,drawtext=fontfile=$font:textfile=$work/author.txt:fontsize=34:fontcolor=white@0.85:x=56:y=h-110"
  fi
  ff -f lavfi -i "gradients=s=640x640:c0=$4:c1=$5:x0=0:y0=0:x1=640:y1=640:d=1$text" -frames:v 1 "$1/cover.jpg"
}

# book FOLDER "TITLE" "AUTHOR" "NARRATOR" "GENRE" MINUTES CHAPTERS YEAR COLOUR1 COLOUR2 "DESCRIPTION" [SERIES PART]
book() {
  dir="$out/$1"; mkdir -p "$dir"
  ms=$(($6 * 60000)); n=$7; each=$((ms / n))
  {
    echo ';FFMETADATA1'
    i=0
    while [ $i -lt "$n" ]; do
      printf '[CHAPTER]\nTIMEBASE=1/1000\nSTART=%d\nEND=%d\ntitle=Chapter %d\n' $((i * each)) $(((i + 1) * each)) $((i + 1))
      i=$((i + 1))
    done
  } > "$work/ch.txt"
  set -- "$@" "" ""
  series=${12}; part=${13}
  album="$2"; [ -n "$series" ] && album="$2: $series, Book $part"
  ff -f lavfi -i "sine=f=180:sample_rate=22050,volume=0.05" -i "$work/ch.txt" -t $(($6 * 60)) \
     -map 0:a -map_metadata 1 -map_chapters 1 -ac 1 -c:a aac -b:a 24k \
     -metadata title="$2" -metadata artist="$3" -metadata album_artist="$3" -metadata composer="$4" \
     -metadata album="$album" -metadata genre="$5" -metadata date="$8" -metadata comment="${11}" \
     -metadata series="$series" -metadata part="$part" \
     -movflags use_metadata_tags -f mp4 "$dir/$2.m4b"
  cover "$dir" "$2" "$3" "$9" "${10}"
}

book "Mara Quill/The Glass Meridian" "The Glass Meridian" "Mara Quill" "Tomas Reed" \
  "Science Fiction & Fantasy, Space Opera" 74 12 2022 0x1b2a4a 0x5b7fb5 \
  "When the relay station at the edge of the system goes silent, its last engineer must decide who to trust." \
  "The Meridian Cycle" 1
book "Mara Quill/Meridian Falling" "Meridian Falling" "Mara Quill" "Tomas Reed" \
  "Science Fiction & Fantasy, Space Opera" 82 14 2023 0x2a1b4a 0x8a5bb5 \
  "The war comes home to the inner worlds, and the Meridian is the only way out." \
  "The Meridian Cycle" 2
book "Iris Penhallow/Salt and Lanterns" "Salt and Lanterns" "Iris Penhallow" "Nell Arden" \
  "Science Fiction & Fantasy, Epic" 68 10 2021 0x0f3b3a 0x3fa39b \
  "A lighthouse keeper's apprentice finds a lantern that shows ships that sank a century ago." \
  "The Lantern Coast" 1
book "Iris Penhallow/The Lantern Keeper" "The Lantern Keeper" "Iris Penhallow" "Nell Arden" \
  "Science Fiction & Fantasy, Epic" 71 11 2022 0x123048 0x4a8ab0 \
  "The drowned ships are returning, and the coast must choose between salt and fire." \
  "The Lantern Coast" 2
book "Edmund Harrow/A Quiet Death in Wexford Lane" "A Quiet Death in Wexford Lane" "Edmund Harrow" "Clare Voss" \
  "Mystery, Thriller & Suspense, Murder" 58 9 2020 0x3a1414 0x9a4a3a \
  "A retired inspector, a village fete and a body in the vicarage greenhouse."
book "Lena Marsh/The Cartographer's Daughter" "The Cartographer's Daughter" "Lena Marsh" "Ruth Aldane" \
  "Literature & Fiction, Historical Fiction" 64 10 2019 0x4a3a1b 0xc29a5b \
  "Lisbon, 1755: a mapmaker's daughter charts a city that is about to disappear."
book "Priya Okafor/Small Engines of the Universe" "Small Engines of the Universe" "Priya Okafor" "Priya Okafor" \
  "Science & Engineering, Physics" 52 8 2023 0x1b3a2a 0x5bb57f \
  "How the tiniest machines in nature, from molecular motors to stars, keep everything running."
book "Gareth Lowe/The Year Without a Summer" "The Year Without a Summer" "Gareth Lowe" "Martin Hale" \
  "History, Europe" 61 9 2018 0x3a3a3a 0x9a9a8a \
  "A volcano on the other side of the world, a lost harvest and the summer that never came."
book "Ruth Calder/Holding the Line" "Holding the Line" "Ruth Calder" "Ruth Calder" \
  "Biographies & Memoirs, Adventurers, Explorers & Survival" 55 8 2021 0x14304a 0xd0d8e0 \
  "Thirty years with a mountain rescue team, told by the woman who led it."
book "Pip Hollis/The Clockwork Fox" "The Clockwork Fox" "Pip Hollis" "Sam Tarrant" \
  "Children's Audiobooks, Animals" 28 6 2024 0x4a2a0f 0xe8933a \
  "A fox made of brass and springs sets out to find the inventor who wound him up."
book "Agnes Thorne/The Fen Hauntings" "The Fen Hauntings" "Agnes Thorne" "Owen Blake" \
  "Horror, Ghosts" 47 7 2020 0x101418 0x3a4a5a \
  "Seven true-sounding tales from the flat, wet and very dark country of the fens."

# A podcast-style show: short episodes sharing one album, grouped into one tile.
for ep in 1 2 3; do
  case $ep in
    1) t="Ep. 1: Starting Out" ;; 2) t="Ep. 2: Tools of the Trade" ;; 3) t="Ep. 3: Seasons" ;;
  esac
  dir="$out/Field Notes/$t"; mkdir -p "$dir"
  ff -f lavfi -i "sine=f=200:sample_rate=22050,volume=0.05" -t $((20 * 60 + ep * 120)) -ac 1 -c:a libmp3lame -b:a 32k \
     -metadata title="$t" -metadata artist="The Allotment Hour" -metadata album="Field Notes" \
     -metadata genre="Podcast" -metadata date="2025" -metadata comment="Growing things, one plot at a time." \
     "$dir/$t.mp3"
  cover "$dir" "Field Notes" "The Allotment Hour" 0x2a4a1b 0x9ac25b
done

echo "showcase library written to $out"
