# Signing the Android release

The release key already exists on the owner's PC at `%USERPROFILE%\.phonebridge-release\` (`release.jks` and `keystore.properties`). The AI session never reads those files. If it is lost, see `KEY-RECOVERY.md`.

## One-time setup (owner, about 10 minutes)

1. **Back up the key now** (see `KEY-RECOVERY.md`, Prevention): password manager entry holding `release.jks` and both passwords, plus a second copy on an encrypted USB drive.
2. **Get the certificate fingerprint** (type the keystore password when asked):
   `keytool -list -v -keystore "$env:USERPROFILE\.phonebridge-release\release.jks"` and copy the `SHA256:` line. Check it equals the fingerprint of the APK already published (`apksigner verify --print-certs Relay.apk`). If it does not, stop: the update path from the published app is broken.
3. **Create a GitHub environment** named `release` (Settings, Environments) and require yourself as reviewer, so a run needs your click.
4. **Add the secrets and one variable** (replace values; run from the folder that holds `release.jks`; `gh` must be signed in):
   ```
   gh secret set ANDROID_KEYSTORE_BASE64 --env release --body ([Convert]::ToBase64String([IO.File]::ReadAllBytes("release.jks")))
   gh secret set ANDROID_KEYSTORE_PASSWORD --env release
   gh secret set ANDROID_KEY_ALIAS --env release
   gh secret set ANDROID_KEY_PASSWORD --env release
   gh variable set ANDROID_CERT_SHA256 --body "<the SHA256 line>"
   ```
   `gh secret set` without `--body` prompts for the value, so passwords never sit in shell history.
5. Record the fingerprint in `docs/DECISIONS.md`.

## Making a release

Actions, `release-android`, Run workflow, approve it. The job builds, checks with `apksigner` that the APK carries exactly the pinned certificate (it fails otherwise), and uploads the APK with its SHA-256. Attach both to the GitHub release by hand.

## What is not done

Build provenance attestation (GitHub's `attest-build-provenance`) is left out until its commit SHA is pinned and checked. The job runs only by hand, never for a pull request or fork.
