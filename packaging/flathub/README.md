# Flathub submission

Files for the pull request to <https://github.com/flathub/flathub> (branch `new-pr`).
The manifest builds from the `v0.1.0` tag instead of a local directory.

Generate `cargo-sources.json` next to the manifest before submitting (it is not versioned):

```sh
python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json   # see ../flatpak/generate-sources.sh
```

For a new release: tag it, then update `tag` and `commit` in the manifest, regenerate
`cargo-sources.json` and add the `<release>` to the metainfo.
