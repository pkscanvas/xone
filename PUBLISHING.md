# Publish Xone on GitHub

## 1. Transfer and extract the package

Copy `Xone-github-upload-kit.zip` to the other computer and extract it. Inside `Xone-upload-kit`, this guide is accompanied by an `artifacts/` folder containing:

- `Xone-github-source.zip` — public app source, website, both videos, and the deployment workflow.
- `Xone_0.1.0_aarch64.dmg` — the Apple Silicon macOS installer.
- `SHA256SUMS.txt` — checksums for the ZIP and DMG.

Extract the source ZIP. The resulting `Xone` folder is the repository root: it contains `README.md`, `package.json`, `src/`, `src-tauri/`, `website/`, and the hidden `.github/` directory.

**Publish the extracted folder contents.** Uploading only the ZIP to a repository does not unpack the website or install its GitHub Actions workflow. Keep the DMG and ZIP outside the source tree; attach them to a release instead.

## 2. Links are already configured

This package targets your existing repository. No URL replacement is needed:

- Repository: [https://github.com/pkscanvas/xone](https://github.com/pkscanvas/xone)
- Planned website: [https://pkscanvas.github.io/xone/](https://pkscanvas.github.io/xone/)
- Download: [https://github.com/pkscanvas/xone/releases/latest](https://github.com/pkscanvas/xone/releases/latest)

The website address assumes standard GitHub Pages hosting without a custom domain. It becomes available after Pages deployment; the download destination becomes available after publishing the first normal release with the DMG.

The README's video thumbnails, video links, and screenshot use relative paths, so they work without editing account names. GitHub may show a file preview or download for an MP4; the website supplies the full video player.

## 3. Upload the source

The repository `pkscanvas/xone` already exists and is empty. Do not create another repository.

Recommended: install GitHub Desktop on the other computer, sign in to the account with write access, and choose **File → Clone Repository → URL**. Enter `https://github.com/pkscanvas/xone` and select a local destination. Copy the **contents** of the extracted source `Xone` folder into that clone, including hidden files such as `.gitignore` and the `.github/` directory. Keep the clone's `.git` directory intact.

Commit the copied files on `main`, then push the branch to GitHub (the button may say **Publish branch** for the first push). Use `main` because the included website workflow watches it.

Alternatively, without cloning first, open a terminal inside the extracted source `Xone` folder and run the following against the existing empty repository:

```sh
git init -b main
git add .
git commit -m "Publish Xone"
git remote add origin https://github.com/pkscanvas/xone.git
git push -u origin main
```

The remote URL is already filled in. Authenticate through Git/GitHub Desktop when prompted. No Node.js or Rust installation is needed just to publish this prepared source and installer.

Check the repository on GitHub: `README.md` and `.github/workflows/deploy-pages.yml` must be at the expected root paths. Do not place everything under an extra `Xone/` directory. GitHub's browser uploader allows 25 MiB per file and 100 files at a time; GitHub Desktop avoids batching and hidden-directory omissions. The included video files are below Git's 100 MiB per-file limit. See [GitHub's upload documentation](https://docs.github.com/en/repositories/working-with-files/managing-files/adding-a-file-to-a-repository).

## 4. Create the download release

On the repository's GitHub page, open **Releases → Draft a new release**. Create tag `v0.1.0` targeting `main`, title it **Xone 0.1.0**, and attach:

- `Xone_0.1.0_aarch64.dmg`
- `Xone-github-source.zip`
- `SHA256SUMS.txt`

Mention Apple Silicon, the external AI provider requirement, and the included first-launch guide in the release notes. Publish it as a normal release, not a prerelease, and select **Set as the latest release** if offered. The website's download button opens `/releases/latest`, where users select the DMG. Before a release is published, this destination will not provide a download. See [GitHub's release documentation](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository).

If you edit the source ZIP after transfer, regenerate its checksum before uploading `SHA256SUMS.txt`; an unchanged supplied ZIP retains its supplied checksum.

## 5. Host the website with GitHub Pages

1. Open the repository's **Settings → Pages**.
2. Under **Build and deployment → Source**, select **GitHub Actions**.
3. Open **Actions → Deploy Xone website to GitHub Pages → Run workflow** and select `main`. Use the included workflow; no additional template is needed. If the first push ran before Pages was enabled and failed, run it again now.
4. Wait for both **build** and **deploy** to finish successfully.
5. Open the address shown under **Settings → Pages** or the workflow's **github-pages** environment. The expected address is `https://pkscanvas.github.io/xone/`.
6. Add that address to the repository's **About → Website** field and confirm the README's Website link matches it.

The workflow installs the website dependencies, builds the React/Vite site, and publishes `website/dist/client`. It automatically sets `VITE_GITHUB_URL` to this repository. Later pushes to `main` changing `website/` or the workflow redeploy the site. README-only edits do not need a website build. See [GitHub's Pages setup documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site).

## 6. Configured website links

In `website/src/App.jsx`, these values are already filled in:

```js
// # github repo here
const githubRepoUrl = "https://github.com/pkscanvas/xone";
// # download link here
const downloadUrl = "https://github.com/pkscanvas/xone/releases/latest";
```

The download button opens the latest release page, where readers can choose the DMG. No further link edits are needed for this repository. To use a direct installer download instead, set `downloadUrl` to `https://github.com/pkscanvas/xone/releases/latest/download/Xone_0.1.0_aarch64.dmg`; update that URL if the asset filename changes.

The configured constants take priority. Clearing them enables the existing `VITE_GITHUB_URL` and optional `VITE_DOWNLOAD_URL` build-time alternatives. These values contain public URLs only.

Finally, open the deployed site on desktop and mobile: check both video choices, README links, GitHub button, and the download destination.
