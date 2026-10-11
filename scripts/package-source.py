#!/usr/bin/env python3
"""Package only the public source and assets, with reproducible ZIP metadata."""
from pathlib import Path
import os
import zipfile

root = Path(__file__).resolve().parent.parent
output = root / "release" / "Xone-github-source.zip"
root_files = {
    ".gitignore", "LICENSE", "README.md", "PUBLISHING.md", "SECURITY.md", "package.json", "package-lock.json",
    "index.html", "svelte.config.js", "tsconfig.json", "tsconfig.node.json", "vite.config.ts",
}
source_roots = {"src", "src-tauri", "website", ".github", "distribution", "scripts"}
excluded_dirs = {"node_modules", "target", "gen", "dist", "release", "qa"}
allowed_suffixes = {
    ".rs", ".toml", ".lock", ".svelte", ".ts", ".css", ".js", ".jsx", ".mjs",
    ".json", ".html", ".yml", ".yaml", ".xml", ".png", ".jpg", ".jpeg", ".svg",
    ".ico", ".icns", ".mp4", ".woff", ".woff2", ".sh", ".applescript", ".py", ".txt",
}
files = [root / name for name in root_files]
for name in sorted(source_roots):
    for directory, dirs, names in os.walk(root / name):
        dirs[:] = sorted(d for d in dirs if d not in excluded_dirs and not d.startswith("."))
        for filename in sorted(names):
            path = Path(directory) / filename
            if path.is_symlink():
                raise SystemExit("Source packages must not contain symlinks")
            if filename in {"README.md", "LICENSE", ".npmrc"} or (
                not filename.startswith(".") and path.suffix in allowed_suffixes
            ):
                files.append(path)
output.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(files):
        entry = zipfile.ZipInfo("Xone/" + path.relative_to(root).as_posix(), (2026, 1, 1, 0, 0, 0))
        entry.compress_type = zipfile.ZIP_DEFLATED
        entry.create_system = 3
        entry.external_attr = (0o100755 if path.suffix == ".sh" else 0o100644) << 16
        archive.writestr(entry, path.read_bytes())
print(f"Packaged {len(files)} public files into {output.name}")
