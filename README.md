# foundryup: the foundry toolchain installer

*foundryup* installs [the Foundry toolchain](https://github.com/foundry-rs/foundry) from the official
release channels.

## Usage

```bash
curl -L https://raw.githubusercontent.com/foundry-rs/foundryup/HEAD/foundryup-init.sh | bash
foundryup
```

## Documentation

See [**The Foundry Book**](https://getfoundry.sh/) for documentation on installing and using *foundryup*.

## Release verification

Prebuilt releases from `v1.3.0-rc1` onward require a valid Sigstore bundle signed by the
`foundry-rs/foundry` release workflow. Branch, pull request, commit, and local installs are built
from source instead. Passing `--force` explicitly disables release verification.

## Download retries

GitHub downloads, including the bootstrap installer, retry transient HTTP, connection,
and response-body failures up to five times, waiting
1, 2, 4, 8, and 16 seconds between attempts. Set `FOUNDRYUP_MAX_RETRIES` to change the
retry count, or `0` to disable retries; delays remain capped at 16 seconds. A failed
attestation download still aborts installation. Each retry restarts the entire download;
partial files are overwritten. Permanent HTTP failures such as 404 and local file I/O
errors are not retried. Incomplete attestations use the same backoff policy. Git and
Cargo manage their own network operations during source builds.

## Getting help

See [**Getting help**](https://github.com/foundry-rs/foundry#getting-help)
