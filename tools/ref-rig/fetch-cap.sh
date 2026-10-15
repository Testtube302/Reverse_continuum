#!/bin/bash
# fetch-cap.sh CONTAINER NAME - convert a ring.sh capture to PNG and unpack it in ~/aicontinuum-ref/NAME/cp
J="-o BatchMode=yes -J testtube@100.90.1.19 testtube@192.168.122.102"
ssh $J "sudo docker exec $1 bash -c 'cd /work; rm -rf cp cp.tgz; mkdir cp; for f in \$(ls cap); do convert cap/\$f cp/\${f%.miff}.png; done; tar czf cp.tgz cp' && sudo docker cp $1:/work/cp.tgz ~/aic-ref/$2.tgz" || exit 1
D=$HOME/aicontinuum-ref/$2; rm -rf "$D"; mkdir -p "$D"
scp -q ${J% *} "${J##* }:aic-ref/$2.tgz" "$D/" && (cd "$D" && tar xzf "$2.tgz" && ls cp | wc -l)
