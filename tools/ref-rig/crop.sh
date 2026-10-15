#!/bin/bash
# crop.sh SHOT NAME WxH+X+Y [scale%] - crop a fetched reference shot in the container, fetch result
S=$HOME/aicontinuum-ref
J="testtube@192.168.122.102"
ssh -o BatchMode=yes -J testtube@100.90.1.19 $J "sudo docker cp ~/aic-ref/shots/$1.png aic-ref:/work/$1.png 2>/dev/null; sudo docker exec aic-ref convert /work/$1.png -crop $3 +repage -scale ${4:-300}% /work/$2.png && sudo docker cp aic-ref:/work/$2.png ~/aic-ref/shots/$2.png" && scp -q -o BatchMode=yes -J testtube@100.90.1.19 $J:aic-ref/shots/$2.png $S/ref-shots/$2.png && echo "$2.png"
