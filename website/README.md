# Xone project website

Static React/Vite project page for Xone. The generated build is compatible with GitHub Pages and uses relative asset paths.

## Local preview

```bash
npm install
npm run dev
```

## Production build

```bash
npm run build
```

The static Pages artifact is written to `dist/client`.

The configured repository is [https://github.com/pkscanvas/xone](https://github.com/pkscanvas/xone). The planned GitHub Pages address is [https://pkscanvas.github.io/xone/](https://pkscanvas.github.io/xone/); it becomes available after deployment.

## Current release content

The page describes 494 sources, the AI / Open Source AI / Design starter Xones, and cited TL:DR briefings. The hero opens with the October 9 cinematic film (50 seconds), followed by the existing product demo (72 seconds). Both have visible thumbnail selectors; switching stops the previous video and does not autoplay.

Music: “Open Spaces” by Hybridas on Uppbeat: https://uppbeat.io/c/hybridas. The track was supplied by the project owner; retain the visible credit. No license document was supplied with it. Do not redistribute its standalone audio.

GitHub and download buttons stay fully visible with their normal labels. `githubRepoUrl` and `downloadUrl` in `src/App.jsx` are configured for `https://github.com/pkscanvas/xone` and `https://github.com/pkscanvas/xone/releases/latest/download/Xone_0.1.0_aarch64.dmg`. Download actions start the DMG download directly; the footer Releases link still opens the release page. The asset must retain the filename `Xone_0.1.0_aarch64.dmg`. The comments `# github repo here` and `# download link here` mark these values for future edits. If the constants are cleared, `VITE_GITHUB_URL` and `VITE_DOWNLOAD_URL` provide build-time alternatives; the GitHub Pages workflow supplies the repository value automatically.

For the complete transfer, repository upload, release, README links, and GitHub Pages setup steps, see [PUBLISHING.md](../PUBLISHING.md).
