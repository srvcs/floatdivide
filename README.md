# srvcs-floatdivide

Floating-point division microservice for srvcs.cloud.

Concern: **float arithmetic: `a / b`**. The result is a floating-point number
(`f64`) — unlike the integer leaf services, the quotient may have a fractional
part (e.g. `floatdivide(7, 2) == 3.5`).

## API

`GET /` — service identity:

```json
{
  "service": "srvcs-floatdivide",
  "concern": "float arithmetic: a / b",
  "depends_on": ["srvcs-isnumber"]
}
```

`POST /` — compute `a / b`:

```json
{ "a": 7, "b": 2 }
```

Response `200`:

```json
{ "a": 7, "b": 2, "result": 3.5 }
```

Both operands may be integers or floats; they are coerced to `f64`.

### Validation and errors

Each operand is validated by delegating to `srvcs-isnumber` over HTTP (the
single source of truth for "is this a number"):

- `422 {"error": "value is not a number"}` — an operand is not a number.
- `422 {"error": "division by zero"}` — `b` is `0`.
- `503` — `srvcs-isnumber` is unreachable; this service reports itself degraded
  rather than guessing.

## Dependencies

Configured via environment variable:

- `SRVCS_ISNUMBER_URL` — base URL of `srvcs-isnumber`
  (default `http://127.0.0.1:8081`).

## Local checks

```sh
nix flake check -L
nix develop -c sh -euc 'cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test'
nix build .#default -L
```

The Linux container is exposed as `.#container`. On Apple Silicon, use
`linux/arm64` for the practical local check; CI builds the release image on
native `x86_64-linux`.

See [`srvcs/platform`](https://github.com/srvcs/platform) for the shared service
standard and CI workflow.
