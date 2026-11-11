# Monoplan

[Monoplan](https://monoplan.app/) is an e2ee, local-first & collaborative, tasks + calendar app, with conflict resolution backed by Loro. It is optimised for ergonomics and frictionless capture and will work offline.

![Monoplan MacOS screenshot](https://fs.yokoso.golf/media/monoplan-macos-26-11-11.png)

⚠️ ATTN: Monoplan is undergoing extensive and frequent changes, you can use it locally, but you may have to manually export and import data between updates.

## Self-hosting with Docker

```bash
docker run -d \
    -p 8000:8000 \
    -v /srv/monoplan:/data \
    ghcr.io/airdaydev/airday-server:v0.0.1-alpha.15
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
