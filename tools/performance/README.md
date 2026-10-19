# Native-window performance measurements

Contributor-original scripts and fixtures, MPL-2.0. These scripts automate an **X11 development
benchmark** with `xdotool`; they do not ship in the application. No commercial sample model,
profile catalogue or material table is used. Python `psutil` is a measurement-tool dependency,
not a BeamCraft dependency.

`prepare_store.py` builds 100 identical native plates plus 32 original material and 32 rectangle
profile definitions through the public CLI. Revision/operation IDs can vary on regeneration;
freeze one generated store and reuse it for both binaries. The generated manifest records its
exported bundle hash. `make_stress.py --help` generates a deterministic bounded `BCVIEW01`
polygon-prism bundle to isolate expanded display-buffer retention. The prism fixture is
**display-only**, not a native BIM project or an engineering-kernel performance test.

`window_bench.py` launches a real native window, waits for successful CLI completion and ten
seconds of warmup, then records three-second phases. The default sequence is idle, orbit, idle,
refresh (R), resize, idle; actions occur at approximately 10 Hz, process samples at 20 Hz.
Initial window size and MSAA are the application's unchanged defaults. Resize requests cycle
1250..1390 by 800..845 window pixels. Five independent UI processes are used per series.
CPU is process user+system time divided by elapsed wall time: **100% means one logical CPU**.
RSS/PSS are sampled UI-process memory, including a software graphics driver when present;
process-tree peaks also include temporary CLI processes. GPU VRAM is not measured.

For the large display fixture, `--settle-refresh` additionally waits for both initial/refresh
CLI call sets to complete and ten further seconds before measuring post-refresh idle. It
records all memory samples during that wait in `refresh_settle`. A three-second refresh
activity phase alone does not establish completion of this asynchronous work.

`cli_wrapper.py` delegates to the selected real CLI and records exit status, wall time and child
CPU/RSS. Its explicitly selected display-fixture mode serves the generated bundle instead;
that mode measures UI loading/rendering/memory, not store inspection. The adapter writes the
same documented bounded envelope used by the production CLI. `summarize.py` preserves all
five samples, min/max and medians. CLI wall sums exclude gaps and UI decoding/upload: **they
are not end-to-end refresh latency**.

A reproducible Linux container is described in `Dockerfile`. The image pins the official Rust
image digest; apt package versions and the actual driver are recorded alongside each report.
Build with your normal trusted CA bundle (a build secret, never committed):

```sh
docker build --secret id=proxy_ca,src=/etc/ssl/certs/ca-certificates.crt \
  -t beamcraft-perf:26 -f tools/performance/Dockerfile .
mkdir -p /workspace/beamcraft-perf
cp tools/performance/*.py tools/performance/*.sh /workspace/beamcraft-perf/
docker run -d --name beamcraft-perf-26 --user "$(id -u):$(id -g)" --cpus 2 --memory 7g \
  -v "$PWD:/repo" -v /workspace/beamcraft-perf:/measure \
  -v /etc/ssl/certs/ca-certificates.crt:/run/host-ca.crt:ro \
  -e CARGO_HOME=/measure/cargo -e CARGO_TARGET_DIR=/measure/target \
  -e CARGO_HTTP_SSL_CAINFO=/run/host-ca.crt -e SSL_CERT_FILE=/run/host-ca.crt \
  -e LP_NUM_THREADS=2 -e XDG_RUNTIME_DIR=/tmp/beamcraft-runtime \
  -e XDG_CACHE_HOME=/measure/cache beamcraft-perf:26 tail -f /dev/null
```

Inside the container, start Xvfb once and keep it running. At the baseline commit, before
editing product code:

```sh
mkdir -p /tmp/beamcraft-runtime /measure/results /measure/bin/before
chmod 700 /tmp/beamcraft-runtime
Xvfb :99 -screen 0 1920x1200x24 -nolisten tcp >/measure/results/xvfb.log 2>&1 &
cargo build --locked --release -p beamcraft-ui -p beamcraft-cli -j 2
cp /measure/target/release/beamcraft /measure/target/release/beamcraft-ui /measure/bin/before/
python3 /measure/prepare_store.py --cli /measure/bin/before/beamcraft \
  --output /measure/fixtures/native-100 --count 100
python3 /measure/make_stress.py /measure/fixtures/prisms-1000 --count 1000 --sides 512
sh /measure/measure_series.sh before
```

After applying the change, build with exactly the same command, copy the binaries to
`/measure/bin/after/`, then run `sh /measure/measure_series.sh after` and
`python3 /measure/summarize.py /measure/results --output /measure/results/summary.json`.
Do not compile, run tests or run other benchmarks concurrently with a measurement series.
Keep the same frozen store, display bundle, adapter, driver, resolution and resource limits.
Inspect CLI traces and UI logs; incomplete or failed loads invalidate a run. Cloud/software-GPU
results do not predict the absolute CPU load or VRAM usage of a discrete GPU.
