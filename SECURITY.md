# Security policy

## Supported versions

Security fixes are applied on the default branch (`main`). There is no long-term
stable branch yet; please upgrade to the latest commit or release tag.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security problems.

Report privately via one of:

- [GitHub Security Advisories](https://github.com/kkir/stomatopod/security/advisories/new)
  for this repository (preferred when available)
- Email the maintainer listed in the GitHub profile for
  [kkir/stomatopod](https://github.com/kkir/stomatopod)

Include steps to reproduce, impact, and any suggested fix. You can expect an
acknowledgement when the report is received; timelines depend on severity and
maintainer availability.

## Scope notes

- Self-hosted deployments must set a strong `auth.secret_key` and admin
  password; do not expose an unconfigured instance to the public internet.
- Optional MaxMind GeoLite databases are supplied by the operator; their
  license terms are separate from this project.
