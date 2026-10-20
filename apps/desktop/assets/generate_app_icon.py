#!/usr/bin/env python3
"""Export the approved brand artwork through the installed Tauri CLI (§6).

The committed app-icon.png is the source artwork, not a procedural placeholder.
Run from any directory; this only rebuilds desktop icon sizes and web copies.
Mobile assets are outside the current desktop release and are left untouched.
"""
from pathlib import Path
import shutil
import subprocess
import tempfile


def main() -> None:
    desktop = Path(__file__).resolve().parents[1]
    source = desktop / "assets/app-icon.png"
    cli = desktop / "node_modules/@tauri-apps/cli/tauri.js"
    with tempfile.TemporaryDirectory(prefix="agenttidy-icons-") as temporary:
        output = Path(temporary)
        subprocess.run(["node", str(cli), "icon", str(source), "--output", str(output)], check=True)
        # Keep the common desktop outputs coherent with the ICNS. The CLI also
        # creates mobile folders, which are intentionally not copied here.
        for icon in output.iterdir():
            if icon.is_file() and icon.suffix in {".png", ".icns", ".ico"}:
                shutil.copyfile(icon, desktop / "src-tauri/icons" / icon.name)
        brand = desktop / "public/brand"
        brand.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(output / "icon.png", brand / "agenttidy-icon.png")
        shutil.copyfile(output / "32x32.png", desktop / "public/favicon.png")
    print("Exported desktop icons and web brand assets from", source)


if __name__ == "__main__":
    main()
