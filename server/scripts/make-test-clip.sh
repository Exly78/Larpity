#!/bin/sh
# Makes a 1-minute 720p 15 fps H.264 test clip in the same format the phone
# sends (Annex B, keyframe every 2 s), for fake-phone to stream. The picture has
# a running clock, which makes delay easy to see.
#
#   scripts/make-test-clip.sh [OUT.h264]
set -eu
out="${1:-test-clip.h264}"
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=1280x720:rate=15" -t 60 \
  -c:v libx264 -profile:v baseline -level 3.1 -pix_fmt yuv420p \
  -g 30 -bf 0 -b:v 1500k -maxrate 1500k -bufsize 1500k \
  -f h264 "$out"
echo "wrote $out"
