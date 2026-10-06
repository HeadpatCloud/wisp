FROM debian:trixie-slim

# The host keys are made when the package is installed, so they belong to the image.
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        openssh-server socat \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash wisp \
    && echo wisp:wisptest | chpasswd \
    && mkdir -p /etc/wisp/keys /run/sshd \
    && ssh-keygen -q -t ed25519 -N '' -C wisp-test-ed25519 -f /etc/wisp/keys/id_ed25519 \
    && ssh-keygen -q -t rsa -b 3072 -N '' -C wisp-test-rsa -f /etc/wisp/keys/id_rsa \
    && ssh-keygen -q -t ed25519 -N wisptest -C wisp-test-ed25519-enc \
        -f /etc/wisp/keys/id_ed25519_enc \
    && install -d -m 700 -o wisp -g wisp /home/wisp/.ssh \
    && cat /etc/wisp/keys/*.pub > /home/wisp/.ssh/authorized_keys \
    && chown wisp:wisp /home/wisp/.ssh/authorized_keys \
    && chmod 600 /home/wisp/.ssh/authorized_keys \
    && printf '%s\n' \
        'PasswordAuthentication yes' \
        'KbdInteractiveAuthentication no' \
        'AllowTcpForwarding yes' \
        'GatewayPorts no' \
        'LogLevel DEBUG1' \
        > /etc/ssh/sshd_config.d/wisp.conf

COPY openssh/sshd_rekey.conf /etc/wisp/sshd_rekey.conf
COPY --chmod=755 openssh/start.sh /usr/local/bin/start

EXPOSE 22 2222
ENTRYPOINT ["start"]
