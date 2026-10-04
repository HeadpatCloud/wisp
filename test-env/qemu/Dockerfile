FROM debian:stable-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        qemu-system-x86 socat \
    && rm -rf /var/lib/apt/lists/*

COPY --chmod=755 qemu/start.sh /usr/local/bin/start

EXPOSE 5900
ENTRYPOINT ["start"]
