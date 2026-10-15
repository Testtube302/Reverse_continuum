#!/bin/bash
# ring.sh - capture the centre 400x400 continuously into /work/cap, keep the last ~8 s, stop when /work/stop exists
cd /work; rm -rf cap stop; mkdir cap; n=0
while [ ! -e stop ]; do
  import -window root -crop 400x400+312+184 cap/$(date +%s%N).miff
  n=$((n+1))
  if [ $((n % 100)) = 0 ]; then
    cut=$(( $(date +%s%N) - 8000000000 ))
    for f in cap/*.miff; do b=${f#cap/}; [ ${b%.miff} -lt $cut ] && rm -f $f; done
  fi
done
