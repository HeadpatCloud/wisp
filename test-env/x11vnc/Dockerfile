FROM debian:trixie-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        x11vnc xvfb \
        xterm xfonts-base \
        xdotool x11-utils x11-xserver-utils x11-apps xclip imagemagick \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir /etc/wisp \
    && x11vnc -storepasswd wisptest /etc/wisp/passwd

COPY --chmod=755 pattern.sh /usr/local/bin/pattern
COPY --chmod=755 x11vnc/start.sh /usr/local/bin/start

ENV DISPLAY=:1
EXPOSE 5900
ENTRYPOINT ["start"]
