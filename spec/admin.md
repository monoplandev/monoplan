# Admin API

The admin API exposes coarse server metadata only. It must never expose encrypted
payloads, credentials, password material, recovery material, or decrypted user
content.

## Authentication and availability

- `admin_password_hash` is an optional server configuration value containing an
  Argon2id PHC string.
- When it is absent, admin routes are not mounted and therefore return `404`.
- When it is present, callers authenticate with
  `Authorization: Bearer <admin-password>` over TLS.
- Missing or invalid credentials return JSON with `401 Unauthorized`.
- The plaintext admin password must never be stored or logged.

Generate a hash by piping one password line to:

```sh
monoplan-server hash-admin-password
```

## `GET /admin/stats`

Returns JSON:

```json
{
  "accounts": 1,
  "devices": 3
}
```

`accounts` and `devices` are counts of the current rows in their respective
tables. Device revocation deletes the device row, so `devices` excludes revoked
devices.

e.g. curl -v -H 'Authorization: Bearer <pw>' https://alpha.monoplan.app/admin/stats
