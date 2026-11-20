# Monoplan

[Monoplan](https://monoplan.app/) is an e2ee, local-first & collaborative, tasks + calendar app, with conflict resolution backed by Loro. It is optimised for ergonomics and frictionless capture and will work offline.

<img width="2296" height="1466" alt="macos-app-nov-19" src="https://github.com/user-attachments/assets/88e47c0e-5ecd-4dba-8632-76369bd7904f" />

⚠️ ATTN: Monoplan is undergoing extensive and frequent changes, you can use it locally, but you may have to manually export and import data between updates.

## Self-hosting with Docker

```bash
docker run -d \
    -p 8000:8000 \
    -v /srv/monoplan:/data \
    ghcr.io/monoplandev/monoplan-server:v0.0.1-alpha.16
```

```
📁 monorepo
├── 📂 apple        → macOS & iOS apps, Swift package over the core FFI
├── 📂 cli          → CLI app & daemon
├── 📂 core         → Rust shared core (CRDT, E2EE, sync), plus wasm (core/web) and FFI (core/ffi) bindings
├── 📂 crates       → Rust support crates (protocol, storage-sqlite)
├── 📂 deploy       → Dockerfile, systemd units, bootstrap & CI scripts
├── 📂 js           → JavaScript packages (config, core, tracer) and web app
├── 📂 scripts      → Dev helper scripts
├── 📂 server       → Sync server
├── 📂 spec         → Specs & plans
└── 📂 telemetry    → Jaeger & OTLP collector for dev envs (wip)
```

## Development Requirements

- Rust
- Bun
- sqlx-cli: (`cargo install sqlx-cli`)
- caddy with cloudflare plugin:

```
go install github.com/caddyserver/xcaddy/cmd/xcaddy@latest
export PATH="$PATH:$(go env GOPATH)/bin" >> .zshenv
xcaddy build --with github.com/caddy-dns/cloudflare
mv caddy /usr/local/bin
```

## Development Setup (MacOS & Arch Linux)

```bash
bun install
bun run config
process-compose up
```
