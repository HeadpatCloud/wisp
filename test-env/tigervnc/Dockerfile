FROM debian:trixie-slim

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        tigervnc-standalone-server tigervnc-tools \
        twm xterm xfonts-base \
        xdotool x11-utils x11-xserver-utils x11-apps xclip imagemagick \
        openssl \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir /etc/wisp \
    && printf 'wisptest\n' | tigervncpasswd -f > /etc/wisp/passwd \
    && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=wisp-test \
        -addext subjectAltName=DNS:localhost,IP:127.0.0.1 \
        -keyout /etc/wisp/key.pem -out /etc/wisp/cert.pem \
    && useradd -m wisp \
    && echo wisp:wisptest | chpasswd

COPY tigervnc/twmrc /root/.twmrc
COPY --chmod=755 pattern.sh /usr/local/bin/pattern
COPY --chmod=755 tigervnc/start.sh /usr/local/bin/start

ENV DISPLAY=:1
EXPOSE 5900
ENTRYPOINT ["start"]
