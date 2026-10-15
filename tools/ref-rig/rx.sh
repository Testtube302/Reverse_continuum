#!/bin/bash
# rx.sh "<commands run in container>" [shot-name]  - run in container, optionally capture + fetch screen
J="-o BatchMode=yes -J testtube@100.90.1.19 testtube@192.168.122.102"
LOCAL=${REF_SHOTS:-$HOME/aicontinuum-ref/shots}
ssh $J "sudo docker exec ${C:-aic-ref} bash -c '$1'"
if [ -n "$2" ]; then
  ssh $J "sudo docker exec ${C:-aic-ref} import -window root /work/$2.png && sudo docker cp ${C:-aic-ref}:/work/$2.png ~/aic-ref/shots/$2.png" && \
  scp -q ${J% *} ${J##* }:aic-ref/shots/$2.png $LOCAL/$2.png && echo "fetched $2.png"
fi
