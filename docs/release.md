# Packaging and releases

peek-rs ships as **Peek RS.app** (bundle id `com.getpeek.rs`). The name and id differ from the
Tauri app's (`Peek.app`, `com.getpeek.dev`) so the two install side by side; both read the same
`~/peek`.

## Install locally

```
./scripts/package.sh                 # release build, sign, install to /Applications (or ~/Applications)
./scripts/package.sh --no-install    # leave the bundle at target/Peek RS.app
./scripts/package.sh --sign "ID"     # sign with this identity
./scripts/package.sh --adhoc         # sign ad-hoc
```

The script builds `peek` with `cargo build --release`, writes `Info.plist` by hand and renders
the `.icns` from `crates/peek-ui/src/dock_icon/midnight.png` with `sips` and `iconutil`. It
signs with the first identity it finds on the keychain (`--sign`, then `PEEK_SIGN_IDENTITY`,
then Developer ID Application, then Apple Development, then ad-hoc). The script is adapted
from coworker's `scripts/package.sh`.

Things that are not obvious from the script:

- **The installed app saves.** A Dock or Spotlight launch cannot pass `--write`, so the
  bundle's `LSEnvironment` sets `PEEK_PERSISTENCE=write`, which `Launch::from_environment`
  takes as the default. An explicit `--read-only` still wins. `cargo run` has no such
  variable and stays read-only unless you pass `--write`. The guards in
  [status.md](status.md#persistence-is-opt-in) apply either way.
- **No PATH is baked in.** A Dock launch gets a bare `PATH`, but the only thing Peek spawns is
  the ACP agent, and `peek-acp/src/shell_path.rs` resolves it against the login shell's
  `PATH`.
- **No `peek://` scheme.** The Rust app does not handle deep links, and claiming the scheme
  would take it from the Tauri app.
- **Hardened runtime, no entitlements.** A real identity signs with `--options runtime
  --timestamp`, which is what notarization requires. Peek neither JITs nor loads third-party
  libraries, and the agent's own binaries are separate processes the runtime does not
  constrain.

## Cut a release

1. Bump `[workspace.package] version` in the root `Cargo.toml` and commit.
2. `git tag -a vX.Y.Z` (the annotation becomes the release body; a lightweight tag gets only
   GitHub's generated notes) and `git push origin vX.Y.Z`.

`.github/workflows/release-macos.yml` then runs on `macos-latest` (arm64). It checks that the
tag matches the workspace version, then imports the Developer ID certificate into a temporary
keychain and runs `./scripts/package.sh --no-install --sign …`. It notarizes and staples the
app, builds a DMG that holds the app and an `/Applications` link, and signs, notarizes and
staples the DMG too. It verifies both with `codesign`, `spctl` and `stapler`, then publishes
`Peek-RS-<version>-aarch64.dmg` and `Peek-RS-<version>-aarch64.app.zip` to a GitHub release.

`workflow_dispatch` runs the same build without releasing. The two files are then attached to
the run as the `macos-arm64` artifact.

The Tauri updater (`latest.json`, `.tar.gz.sig`) is not reproduced; peek-rs has no updater.

## Secrets

The workflow reads these from the repository's `default` **environment**, as getpeek/peek does.
GitHub never returns a secret's value, so they have to be set again from the original files:

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12`'s export password |
| `APPLE_API_KEY` | base64 of the App Store Connect API key `AuthKey_<id>.p8` |
| `APPLE_API_KEY_ID` | that key's id |
| `APPLE_API_ISSUER_ID` | the App Store Connect issuer UUID |

```
gh api -X PUT repos/getpeek/peek-rs/environments/default
base64 -i DeveloperID_Application.p12 | gh secret set APPLE_CERTIFICATE -R getpeek/peek-rs --env default
gh secret set APPLE_CERTIFICATE_PASSWORD -R getpeek/peek-rs --env default
base64 -i AuthKey_XXXXXXXXXX.p8 | gh secret set APPLE_API_KEY -R getpeek/peek-rs --env default
gh secret set APPLE_API_KEY_ID -R getpeek/peek-rs --env default --body XXXXXXXXXX
gh secret set APPLE_API_ISSUER_ID -R getpeek/peek-rs --env default --body <issuer-uuid>
```

The repository is public. That is safe, because environment secrets are never handed to
workflows triggered from forks, and this workflow runs only on tags and manual dispatch. The
`.p12` can be exported from Keychain Access (My Certificates → the Developer ID Application
entry → Export). The `.p8` downloads from App Store Connect only once, so reuse the file
getpeek/peek was set up with, or make a new key.
