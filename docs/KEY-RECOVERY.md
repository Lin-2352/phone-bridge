# If a signing key is lost or leaked

Plain steps for the owner. Nothing here needs a signing key to read.

## What each key does

| | Windows (`Relay.exe`, installer) | Android (APK) |
|---|---|---|
| What the key proves | Who published the file; Windows SmartScreen reputation | The APK is an update of the app already installed |
| Lost key, effect on installed copies | None. They keep working. | None. They keep working. |
| Lost key, effect on updates | None. Sign the next release with a new certificate. | **Nobody can update.** Android refuses an APK signed by a different key. |
| Cost of recovery | SmartScreen reputation starts again (weeks of warnings) | Users uninstall and reinstall under a new key, and pair again |

A passkey (Windows Hello, Android) cannot sign files or recover a signing key. It can only protect the password-manager vault where the backup lives.

## Prevention (do this once, before the first signed release)

1. Generate the Android keystore on the owner's own machine. It never goes to the AI session, the repo, or chat.
2. Store the keystore file and both passwords in a password manager unlocked by a passkey. Keep a second copy on an encrypted USB drive stored elsewhere.
3. Write the certificate's SHA-256 fingerprint in the README and in `docs/DECISIONS.md`, so a restored key can be checked against it.
4. Put the keystore in GitHub Actions secrets for CI only. Nobody reads it back from there.
5. Test the restore once: restore from the backup and check `apksigner verify --print-certs` shows the same fingerprint.
6. When Google's developer verification reaches India (2027), register `dev.phonebridge.app` with the same key.

## Windows: key lost, leaked, or the free signing programme says no

1. Stop the release job. Revoke the certificate with whoever issued it (SignPath: tell them).
2. Get a new certificate (re-apply to SignPath, or another route in `docs/DECISIONS.md`).
3. Ship the next release signed with the new one. Installed copies need nothing.
4. Say in the release notes that the publisher name or certificate changed and SmartScreen may warn for a while.

## Android: key lost

1. First try every backup: password manager, USB drive, GitHub secrets (cannot be read back, but a CI run that still signs proves the key exists; ask the owner to export it from a machine that still has it).
2. If a copy is found, verify the fingerprint, restore it, done.
3. If none exists, the app can never be updated in place. Recovery:
   1. Generate a new keystore (and back it up this time).
   2. Publish the app under a **new package name** (reserved fallback: `dev.phonebridge.relay`), because the old package can only be updated by the old key.
   3. Release notes and README: "uninstall the old Relay, install the new one, pair again". Nothing else carries over: pairings and settings live in the old app's private storage.
   4. Keep the old package available for installs but mark it frozen.
4. Ship the migration helper (open item, see PROGRESS): an in-app "Export pairings" file that the new app imports, so reinstalling does not mean pairing every PC again. Build it before the first public release so it is never needed in a panic.

## Key leaked (someone else could sign as Relay)

Treat as lost: Windows steps 1 to 4 above; Android steps 3.1 to 3.4 (a leaked Android key cannot be revoked, so move users to the new package). Rotate every GitHub secret that held it. Publish what happened.

## Both lost at once

Do the Windows and Android sections independently. They do not depend on each other. Order: Android first (users are blocked from updates), then Windows (users are not blocked).
