#!/bin/sh
set -eu
label="$1"
python3 /measure/window_bench.py --ui "/measure/bin/$label/beamcraft-ui" --cli "/measure/bin/$label/beamcraft" --store /measure/fixtures/native-100 --output "/measure/results/$label-native.json" --label "$label-native" > "/measure/results/$label-native.log" 2>&1
python3 /measure/window_bench.py --ui "/measure/bin/$label/beamcraft-ui" --cli "/measure/bin/$label/beamcraft" --store /measure/fixtures/native-100 --fixture /measure/fixtures/prisms-1000 --output "/measure/results/$label-stress.json" --label "$label-stress" --actions idle refresh idle --repetitions 5 --settle-refresh > "/measure/results/$label-stress.log" 2>&1
