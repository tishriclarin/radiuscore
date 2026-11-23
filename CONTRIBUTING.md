# Contributing to RadiusCore

Use [GitHub Issues](https://github.com/tishriclarin/radiuscore/issues) for bugs and feature requests. For questions about deployment or development, see [Support](SUPPORT.md). Report security issues privately using the [security policy](SECURITY.md).

## Prepare a pull request

1. Fork the repository and make a branch in your fork.
2. Keep the change focused. For code changes, add or update a test when the behavior needs coverage.
3. Run the checks listed below. In your pull request, describe the change, its expected behavior, and the checks you ran.
4. Open the pull request against `main`. A maintainer reviews changes before merging.

```sh
cargo fmt --check
cargo test --locked
cargo build --locked --release
```

For schema or accounting changes, run `PGDATABASE=radiuscore_test ./scripts/test-postgres.sh` against a fresh, disposable PostgreSQL 17 database. For Option 82 or RADIUS interoperability changes, see the [lab procedure](docs/option82-interop.md).

Use synthetic data in fixtures and issue reports. Remove NAS shared secrets, credentials, subscriber identifiers, and private network details from logs or packet captures before posting.
