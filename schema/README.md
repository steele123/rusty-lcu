# LCU Swagger Schema

`schema/swagger.json` is the vendored OpenAPI source used by `build.rs`.

Default upstream:

```text
https://raw.githubusercontent.com/dysolix/hasagi-types/main/swagger.json
```

Refresh it with:

```powershell
scripts\update-swagger.ps1
```

or:

```sh
./scripts/update-swagger.sh
```

You can also build against another local schema without replacing this file:

```powershell
$env:LCU_SWAGGER_PATH = "C:\path\to\swagger.json"
cargo check
```

Generated Rust endpoint and model code is emitted into Cargo's `OUT_DIR` at
build time and is not committed.
