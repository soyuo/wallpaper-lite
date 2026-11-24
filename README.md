<p align="center">
  <img src="docs/assets/icon.png" width="128" height="128" alt="wallpaper-lite icon">
</p>

<h1 align="center">wallpaper-lite</h1>

<p align="center">A lightweight live wallpaper app for Windows 10 and 11.</p>

wallpaper-lite shows images and videos behind your desktop icons. It is built with Rust
(a wallpaper host that draws with Direct3D 11) and Tauri, React and TypeScript (the
settings app). The app lives in the system tray, can start when you sign in, and can
back up and restore its settings.

> **Status:** under development, not yet released. The installer builds, but it has
> not been verified on a clean Windows 10 or Windows 11 machine. See
> [release.md](docs/development/release.md#verification-status) for exactly what has
> and has not been checked.

## Build and run

- Development setup and toolchain: [docs/development/setup.md](docs/development/setup.md)
- Running the settings app from source: [apps/ui/README.md](apps/ui/README.md)
- Building the Windows installer: [docs/development/release.md](docs/development/release.md)

## Documentation

- Architecture notes: [docs/architecture](docs/architecture)
- Acceptance records and verification procedures: [docs/development](docs/development)

## Icon

The icon source is `apps/ui/src-tauri/icons/icon.svg`. Run `python scripts/build-icons.py`
to regenerate `icon.ico` (16 to 256 px), `icon.png` and `docs/assets/icon.png` from it.
It needs Python with Pillow and Microsoft Edge or Google Chrome.

## License

BSD 2-Clause. See [LICENSE](LICENSE).
