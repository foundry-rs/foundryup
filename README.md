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

## Getting help

See [**Getting help**](https://github.com/foundry-rs/foundry#getting-help)
