FROM debian:trixie-slim

# python3-xlib: xdotool segfaults on this X server (no XKEYBOARD), pointer.py replaces
# xdotool getmouselocation
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        tightvncserver \
        xterm xfonts-base \
        xdotool x11-utils x11-xserver-utils x11-apps xclip imagemagick \
        python3-xlib \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir /etc/wisp \
    && printf 'wisptest\n' | tightvncpasswd -f > /etc/wisp/passwd

COPY --chmod=755 pattern.sh /usr/local/bin/pattern
COPY --chmod=755 tightvnc/pointer.py /usr/local/bin/pointer
COPY --chmod=755 tightvnc/start.sh /usr/local/bin/start

ENV DISPLAY=:1
EXPOSE 5900
ENTRYPOINT ["start"]
