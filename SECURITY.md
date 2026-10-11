# Security

The distributed release targets macOS on Apple Silicon. Windows and Linux builds are not release-validated.

## Local data

Provider credentials, reading history, and settings are stored in the local SQLite database. On Unix platforms the data directory is restricted to its owner (0700), and the database and existing journals are restricted to their owner (0600). Credentials are not encrypted or stored in the system keychain. Software running as the same user can read them; protect local backups accordingly.

Cloud features send their required inputs to the selected provider. API credentials remain on the configured origin during redirects. Provider HTTP error bodies are not shown or persisted, and transport errors omit request URLs.

## Untrusted content

Article links may open only HTTP or HTTPS addresses without embedded credentials. Feed downloads are limited to 20 MiB. The webview uses a content security policy, and article text is rendered as text rather than executable HTML.

## Dependency scope

The Apple Silicon release uses the patched RSS parser and TLS dependencies in `Cargo.lock`. The cross-platform lockfile still includes the Linux-only GTK dependency `glib 0.18.5`, affected by RUSTSEC-2024-0429; this dependency is not in the macOS build. A Linux release requires resolving that upstream dependency and separate validation. Upstream `unic-*` crates and `proc-macro-error` also have maintenance advisories; these are not equivalent to confirmed exploitable vulnerabilities in this release.

Run dependency audits and source/artifact secret scans before each release. Never publish application-data directories or local configuration. Keep the source ZIP and installer separate from development files and test data.
