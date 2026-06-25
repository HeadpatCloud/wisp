# Security Policy

## Reporting a vulnerability

Please report security issues privately, not through public GitHub issues.

- Preferred: open a private report through GitHub Security Advisories ("Report a vulnerability"
  under this repository's Security tab).

Include what you found, how to reproduce it, and the impact. I'll acknowledge as soon as I can
and keep you updated on a fix.

## Especially relevant areas

Wisp stores SSH keys, passwords, and other secrets and connects to remote hosts, so issues here
matter most:

- The encrypted secret vault and key handling
- SSH host-key verification (trust-on-first-use)
- TLS / FTPS certificate validation
- The auto-updater signature checks
- Anything that could leak credentials or lead to code execution

## Supported versions

Fixes land on the latest release. Please reproduce on the most recent version before reporting.
