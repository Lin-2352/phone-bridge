# Contributing

Bug reports and pull requests are welcome. For anything large, open an issue first so we can agree on the approach.

## Reporting a bug

Include your phone model and Android version, your Windows version, the steps that trigger it, and what you expected. Logs help: on Windows they are in `data\logs` beside `Relay.exe` for a portable copy, and on the phone you can export them from Settings. Check them for anything private before attaching them.

Please report security problems privately, as described in [SECURITY.md](SECURITY.md).

## Pull requests

- Run the checks in the [README](README.md#build-from-source). CI runs the same ones.
- Add a test that fails without your change, wherever the logic can be tested off the device.
- If you change something on a device, say what you tried it on. Android makers differ a lot, so "works on my phone" is useful information as long as the phone is named.
- Keep changes focused. Update `docs/THREAT_MODEL.md` if your change adds a way in, and `docs/WIRE_PROTOCOL.md` if it changes messages.

## Ground rules for the code

- **Never copy from GPL or AGPL projects** such as KDE Connect, Deskflow, Sunshine or RustDesk. This project is `MIT OR Apache-2.0`.
- **The PC can only ask.** Anything sensitive on the phone needs the phone's own consent, and the UI must always show what's really running.
- **Stay light in the background.** Relay runs all day, so measure before adding anything that wakes up often.

By contributing, you agree that your work is licensed under the same terms as the project.
