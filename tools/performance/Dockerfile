FROM rust:1.96.0-slim-trixie@sha256:c37af730be4fd8104cbf9aedbd6ab259e51ca2d5437817a0f8680edf66ac6c28
RUN --mount=type=secret,id=proxy_ca,required=true \
    SSL_CERT_FILE=/run/secrets/proxy_ca apt-get update && \
    SSL_CERT_FILE=/run/secrets/proxy_ca apt-get install -y --no-install-recommends \
      xvfb xauth xdotool mesa-vulkan-drivers vulkan-tools libxkbcommon-x11-0 \
      libxcb-cursor0 libwayland-client0 libxrandr2 libxi6 libxcursor1 libxinerama1 \
      git python3 python3-psutil procps time strace pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*
RUN --mount=type=secret,id=proxy_ca,required=true \
    SSL_CERT_FILE=/run/secrets/proxy_ca rustup component add clippy rustfmt && \
    SSL_CERT_FILE=/run/secrets/proxy_ca rustup target add wasm32-unknown-unknown
WORKDIR /repo
