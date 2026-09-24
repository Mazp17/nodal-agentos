
## Install

1. Download `Nodal_<version>_universal.dmg` (Apple Silicon and Intel), open it and drag Nodal to Applications.
2. Nodal is not notarized by Apple, so macOS blocks the first launch ("Nodal is damaged" or "cannot be opened"). Remove the quarantine flag once:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Nodal.app
   ```

Optional: verify the download with `shasum -a 256 -c Nodal_<version>_universal.dmg.sha256`.
