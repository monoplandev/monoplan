# Monoplan

[Monoplan](https://monoplan.app/) is an e2ee, local-first, tasks/reminders app i.e. todo list, with conflict resolution backed by Loro. It is optimised for ergonomics and frictionless capture and will work offline.

https://github.com/user-attachments/assets/8b64c3d5-279a-4467-85c1-b5930639a0e9

⚠️ ATTN: Monoplan is undergoing extensive and frequent changes, you can use it locally, but you may have to manually export and import data between updates.

## Self-hosting with Docker

```bash
docker run -d \
    -p 8000:8000 \
    -v /srv/monoplan:/data \
    ghcr.io/airdaydev/airday-server:v0.0.1-alpha.13
```

```
📁 monorepo
├── 📂 cli  → CLI app
├── 📂 ios          → iOS app placeholder
├── 📂 js           → JavaScript packages and web app
├── 📂 core         → Rust shared core
├── 📂 crates         → Rust packages
├── 📂 server       → Server
├── 📂 sqlite       → Sqlite migration & notes
└── 📂 telemetry    → Jaeger & OTLP collector for dev envs (may upgrade to clickstack)
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
