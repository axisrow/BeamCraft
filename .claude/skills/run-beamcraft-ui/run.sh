#!/usr/bin/env bash
# Launch beamcraft-ui headless on Linux (Xvfb + Mesa lavapipe) against a sample store.
#
#   run.sh setup            install the runtime packages (Vulkan software driver, X tools)
#   run.sh build            release build of beamcraft (CLI) and beamcraft-ui
#   run.sh store DIR        author a sample frame into the empty absolute directory DIR
#   run.sh start DIR        start Xvfb on :99 and the UI on store DIR, focused, 1600x1000
#   run.sh shot FILE        screenshot the whole virtual screen to FILE
#   run.sh click X Y        click at screen coordinates (slow enough for egui to see it)
#   run.sh type TEXT        type into the focused egui text field
#   run.sh paste TEXT       paste TEXT into the focused field (long or non-ASCII text)
#   run.sh key KEY          send one key (e.g. f to fit, r to reload)
#   run.sh stop             stop the UI and Xvfb
set -euo pipefail

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
BIN="$ROOT/target/release"
export DISPLAY="${DISPLAY_NUM:-:99}"

window() { xdotool search --name '^BeamCraft$' | head -1; }

case "${1:-}" in
setup)
  pkgs="mesa-vulkan-drivers libvulkan1 libxkbcommon-x11-0 xvfb xdotool xclip x11-utils imagemagick"
  apt-get install -y $pkgs >/dev/null 2>&1 || { apt-get update >/dev/null && apt-get install -y $pkgs >/dev/null; }
  ;;
build)
  cargo build --release --manifest-path "$ROOT/Cargo.toml" -p beamcraft-cli -p beamcraft-ui
  ;;
store)
  st="${2:?absolute store directory}"
  mkdir -p "$st"
  b="$BIN/beamcraft"
  digest() { grep -o '"digest":"[0-9a-f]*"' | head -1 | cut -d'"' -f4; }
  mat=$("$b" define-material --store "$st" --density 7850 --grade S355 | digest)
  col=$("$b" define-profile --store "$st" --width 300 --height 300 --designation "RHS 300x300" | digest)
  bm=$("$b" define-profile --store "$st" --width 200 --height 400 --designation "RECT 200x400" | digest)
  for x in 0 6000 12000; do
    for y in 0 5000; do
      "$b" create-member --store "$st" --kind column --start "$x,$y,0" --end "$x,$y,4000" \
        --profile-digest "$col" --material-digest "$mat" --name "C-$x-$y" >/dev/null
      "$b" create-plate --store "$st" --width 500 --length 500 --thickness 25 \
        --translation "$((x - 250)),$((y - 250)),-25" --material-digest "$mat" --name BP \
        --hole-round "60,60,26;440,60,26;60,440,26;440,440,26" >/dev/null
    done
    "$b" create-member --store "$st" --kind beam --start "$x,0,4200" --end "$x,5000,4200" \
      --profile-digest "$bm" --material-digest "$mat" --name "B-$x" >/dev/null
  done
  for y in 0 5000; do
    "$b" create-member --store "$st" --kind beam --start "0,$y,4200" --end "12000,$y,4200" \
      --profile-digest "$bm" --material-digest "$mat" --name "BL-$y" >/dev/null
  done
  echo "sample store ready: $st (17 elements)"
  ;;
start)
  st="${2:?absolute store directory}"
  log="${UI_LOG:-$st/../ui.log}"
  pgrep -x Xvfb >/dev/null || { Xvfb "$DISPLAY" -screen 0 1600x1000x24 >/dev/null 2>&1 & }
  for _ in $(seq 1 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.2; done
  WGPU_BACKEND=vulkan VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
    nohup "$BIN/beamcraft-ui" --store "$st" >"$log" 2>&1 &
  for _ in $(seq 1 30); do [ -n "$(window)" ] && break; sleep 0.5; done
  w="$(window)"
  [ -n "$w" ] || { echo "UI window did not appear; see $log" >&2; exit 1; }
  xdotool windowsize "$w" 1600 1000 windowmove "$w" 0 0
  # No window manager runs, so keyboard focus must be given explicitly or text fields stay empty.
  # Right after the window maps, X can refuse the focus (BadMatch); retry briefly.
  for _ in 1 2 3 4 5; do xdotool windowfocus --sync "$w" 2>/dev/null && break; sleep 1; done
  # The model loads in the background and the Create form sits higher until it has, so wait for
  # the viewport to show the renderer's clear colour, exactly (5, 8, 13); before the first paint
  # the screen is black and with no model it is panel grey. A store without a revision never
  # loads; give up after about 20 s.
  loaded=no
  for _ in $(seq 1 40); do
    rgb=$(import -window root -crop 1x1+1000+980 \
      -format '%[fx:int(255*r+0.5)] %[fx:int(255*g+0.5)] %[fx:int(255*b+0.5)]' info: 2>/dev/null || true)
    if [ "$rgb" = "5 8 13" ]; then loaded=yes; break; fi
    sleep 0.5
  done
  sleep 1
  echo "UI up on $DISPLAY (window $w, model loaded: $loaded), log: $log"
  ;;
shot)
  import -window root "${2:?output png}"
  ;;
click)
  # egui misses a press and release in the same frame; keep them apart.
  xdotool mousemove "${2:?x}" "${3:?y}"; sleep 0.2
  xdotool mousedown 1; sleep 0.15; xdotool mouseup 1; sleep 0.3
  ;;
type)
  # `--` so text such as "-5" is typed, not parsed as an option.
  xdotool type --delay 80 -- "${2:?text}"; sleep 0.3
  ;;
paste)
  # xdotool remaps a key for every non-ASCII letter and drops or reorders them when typing fast,
  # so long or Cyrillic text goes through the clipboard instead.
  printf %s "${2:?text}" | xclip -selection clipboard -i
  xdotool key ctrl+v; sleep 0.5
  ;;
key)
  xdotool key "${2:?key}"; sleep 0.5
  ;;
stop)
  pkill -x beamcraft-ui || true
  pkill -x Xvfb || true
  # Wait until it has exited, or the next `start` would find the dying server and skip launching.
  for _ in $(seq 1 50); do pgrep -x Xvfb >/dev/null || break; sleep 0.2; done
  ;;
*)
  sed -n '2,14p' "$0"; exit 2
  ;;
esac
