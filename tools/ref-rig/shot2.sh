#!/bin/bash
# shot2.sh NAME - grab both screens at the same moment into ~/aicontinuum-ref/shots/NAME_o.png / NAME_u.png
J="-o BatchMode=yes -J testtube@100.90.1.19 testtube@192.168.122.102"
ssh $J "sudo docker exec aic-wine5 import -window root /work/$1_o.png & sudo docker exec aic-ref env DISPLAY=:99 import -window root /work/$1_u.png; wait; sudo docker cp aic-wine5:/work/$1_o.png ~/aic-ref/shots/; sudo docker cp aic-ref:/work/$1_u.png ~/aic-ref/shots/"
scp -q ${J% *} "${J##* }:aic-ref/shots/$1_[ou].png" ~/aicontinuum-ref/shots/
