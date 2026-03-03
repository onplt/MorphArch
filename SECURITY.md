# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.4.x   | Yes       |
| < 0.4   | No        |

## Reporting a Vulnerability

If you discover a security vulnerability in MorphArch, please report it
responsibly.

## Scope

MorphArch is a local CLI tool that reads Git repositories and stores data in a
local SQLite database (`~/.morpharch/morpharch.db`). It does not expose network
services or handle authentication credentials.

Security concerns most relevant to this project include:

- Path traversal when scanning repositories
- SQL injection in database queries
- Denial of service through maliciously crafted Git objects or source files
- Dependency vulnerabilities in third-party crates
