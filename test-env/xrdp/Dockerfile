FROM debian:trixie-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        xrdp xorgxrdp \
        twm xterm xfonts-base \
        xdotool xclip x11-utils x11-xserver-utils x11-apps imagemagick \
        openssl \
    && rm -rf /var/lib/apt/lists/*

RUN rm /etc/xrdp/cert.pem /etc/xrdp/key.pem \
    && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=wisp-test \
        -addext subjectAltName=DNS:localhost,IP:127.0.0.1 \
        -keyout /etc/xrdp/key.pem -out /etc/xrdp/cert.pem \
    && useradd -m -s /bin/sh wisp \
    && echo wisp:wisptest | chpasswd

COPY --chown=wisp:wisp tigervnc/twmrc /home/wisp/.twmrc
COPY --chmod=755 pattern.sh /usr/local/bin/pattern
COPY --chmod=755 xrdp/startwm.sh /etc/xrdp/startwm.sh
COPY --chmod=755 xrdp/start.sh /usr/local/bin/start

EXPOSE 3389
ENTRYPOINT ["start"]
